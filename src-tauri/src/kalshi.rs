//! Integração de leitura com a Kalshi, igual ao `server.js`: chamadas privadas
//! assinadas (RSA-PSS), paginação, metadados públicos de mercados/eventos com
//! cache persistido, enriquecimento de posições e liquidações e as rotas.
//! Somente leitura: nada aqui envia ordens.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use rusqlite::OptionalExtension;
use rusqlite::types::ToSql;
use serde_json::{Map, Value, json};
use tauri::http::StatusCode;

use crate::api::{ApiResponse, Estado};
use crate::db::{Db, agora};
use crate::kalshi_dados as kd;
use crate::net::{self, ErroHttp};
use crate::{history, js, ledger, titles};

const BASE: &str = "https://external-api.kalshi.com";
const TTL_ENDPOINT: Duration = Duration::from_secs(45);
const TTL_META_MS: f64 = 6.0 * 60.0 * 60.0 * 1000.0;
const REPETIR_INCOMPLETA_MS: f64 = 10.0 * 60.0 * 1000.0;
const PRAZO_PRIVADO: Duration = Duration::from_secs(25);
const PRAZO_PUBLICO: Duration = Duration::from_secs(30);

#[derive(Clone)]
struct Liquidacao {
    assinatura: String,
    linha: Value,
    em: f64,
}

/// Caches em memória por usuário (o cache de liquidações também fica no SQLite).
#[derive(Default)]
pub struct EstadoKalshi {
    endpoint: Mutex<HashMap<String, (Instant, Value)>>,
    liquidacoes: Mutex<HashMap<String, HashMap<String, Liquidacao>>>,
    revisados: Mutex<HashMap<String, HashMap<String, String>>>,
}

impl EstadoKalshi {
    fn endpoint_get(&self, chave: &str) -> Option<Value> {
        let mapa = self.endpoint.lock().ok()?;
        mapa.get(chave).filter(|(em, _)| em.elapsed() < TTL_ENDPOINT).map(|(_, v)| v.clone())
    }

    fn endpoint_set(&self, chave: String, valor: Value) {
        if let Ok(mut m) = self.endpoint.lock() {
            m.insert(chave, (Instant::now(), valor));
        }
    }

    /// Tira as respostas vencidas, que `endpoint_get` já não devolve.
    /// Liquidações e revisões ficam: valem a sessão inteira.
    pub fn limpar_vencidos(&self) {
        if let Ok(mut m) = self.endpoint.lock() {
            m.retain(|_, (em, _)| em.elapsed() < TTL_ENDPOINT);
        }
    }

    /// `clearKalshiUserCaches()` (a parte em memória).
    fn limpar_usuario(&self, user_id: &str) {
        if let Ok(mut m) = self.endpoint.lock() {
            m.remove(&format!("{user_id}|positions"));
            m.remove(&format!("{user_id}|settlements"));
        }
        if let Ok(mut m) = self.liquidacoes.lock() {
            m.remove(user_id);
        }
        if let Ok(mut m) = self.revisados.lock() {
            m.remove(user_id);
        }
    }
}

fn agora_ms() -> f64 {
    chrono::Utc::now().timestamp_millis() as f64
}

fn truthy(v: Option<&Value>) -> bool {
    js::truthy(v)
}

fn texto(v: Option<&Value>) -> String {
    if js::truthy(v) { js::string(v) } else { String::new() }
}

fn ou<'a>(a: Option<&'a Value>, b: Option<&'a Value>) -> Option<&'a Value> {
    titles::ou(a, b)
}

// ---------------------------------------------------------------------------
// Cache de metadados (kalshi_metadata_cache)
// ---------------------------------------------------------------------------

/// Começo dos marcadores que tomam o lugar de uma cópia dentro do evento
/// gravado. O caractere nulo nunca aparece num texto da API.
const COPIA: &str = "\u{0}betty:";

/// O evento como vai para o cache: `markets` (igual a `_markets`) e
/// `_milestone` (um dos `_milestones`) viram marcadores na mesma posição,
/// porque as cópias eram quase metade do cache. `expandir_evento` desfaz.
/// A comparação é pelo texto, porque o `==` do `serde_json` ignora a ordem
/// das chaves.
fn compactar_evento(ev: &Value) -> Value {
    let mut ev = ev.clone();
    let Some(o) = ev.as_object_mut() else { return ev };
    let texto = |v: Option<&Value>| v.map(js::para_texto);
    if o.contains_key("markets") && texto(o.get("markets")) == texto(o.get("_markets")) {
        o.insert("markets".into(), Value::String(format!("{COPIA}_markets")));
    }
    if let (Some(m), Some(Value::Array(lista))) = (o.get("_milestone"), o.get("_milestones")) {
        let m = js::para_texto(m);
        if let Some(i) = lista.iter().position(|x| js::para_texto(x) == m) {
            o.insert("_milestone".into(), Value::String(format!("{COPIA}_milestones/{i}")));
        }
    }
    ev
}

/// Põe de volta as cópias que `compactar_evento` trocou por marcadores.
pub(crate) fn expandir_evento(ev: &mut Value) {
    let Some(o) = ev.as_object_mut() else { return };
    if o.get("markets").and_then(Value::as_str) == Some(&format!("{COPIA}_markets"))
        && let Some(m) = o.get("_markets").cloned()
    {
        o.insert("markets".into(), m);
    }
    let indice = o.get("_milestone").and_then(Value::as_str).and_then(|s| s.strip_prefix(COPIA)).and_then(|s| s.strip_prefix("_milestones/"));
    if let Some(m) = indice.and_then(|i| i.parse::<usize>().ok()).and_then(|i| o.get("_milestones")?.get(i).cloned()) {
        o.insert("_milestone".into(), m);
    }
}

/// Compacta uma vez os eventos já gravados no cache (`compactar_evento`).
/// Roda na manutenção da abertura, antes do `VACUUM`. Devolve quantos
/// eventos mudaram e os bytes antes e depois, ou `None` se já tinha rodado.
pub fn compactar_cache_uma_vez(c: &mut rusqlite::Connection) -> Result<Option<(usize, usize, usize)>> {
    const CHAVE: &str = "compact_kalshi_event_cache_v1";
    let feito = c.query_row("SELECT 1 FROM database_maintenance WHERE maintenance_key=?", [CHAVE], |_| Ok(())).optional()?.is_some();
    if feito {
        return Ok(None);
    }
    let linhas: Vec<(String, String)> = {
        let mut s = c.prepare("SELECT cache_key,data_json FROM kalshi_metadata_cache WHERE kind='event'")?;
        let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let (mut mudados, mut antes, mut depois) = (0, 0, 0);
    let tx = c.transaction()?;
    {
        // `updated_at` fica: o prazo de validade do metadado não muda.
        let mut s = tx.prepare("UPDATE kalshi_metadata_cache SET data_json=? WHERE kind='event' AND cache_key=?")?;
        for (chave, texto) in linhas {
            antes += texto.len();
            let Ok(ev) = serde_json::from_str::<Value>(&texto) else {
                depois += texto.len();
                continue;
            };
            let novo = js::para_texto(&compactar_evento(&ev));
            depois += novo.len();
            if novo != texto {
                s.execute(rusqlite::params![novo, chave])?;
                mudados += 1;
            }
        }
    }
    tx.execute("INSERT INTO database_maintenance(maintenance_key,completed_at) VALUES(?,?)", rusqlite::params![CHAVE, agora()])?;
    tx.commit()?;
    Ok(Some((mudados, antes, depois)))
}

/// `cacheGet()`: metadado persistido ainda dentro do TTL de 6 h.
async fn meta_varios(db: &Db, tipo: &'static str, chaves: Vec<String>, com_ttl: bool) -> Result<HashMap<String, Value>> {
    if chaves.is_empty() {
        return Ok(HashMap::new());
    }
    db.ler(move |c| {
        let mut achados = HashMap::new();
        let agora = agora_ms();
        for lote in chaves.chunks(400) {
            let marcas = vec!["?"; lote.len()].join(",");
            let sql = format!("SELECT cache_key,data_json,updated_at FROM kalshi_metadata_cache WHERE kind=? AND cache_key IN ({marcas})");
            let mut s = c.prepare(&sql)?;
            let mut p: Vec<&dyn ToSql> = vec![&tipo];
            p.extend(lote.iter().map(|k| k as &dyn ToSql));
            let rows = s.query_map(p.as_slice(), |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
            for row in rows {
                let (k, d, em) = row?;
                if com_ttl {
                    match js::data_ms(&em) {
                        Some(em) if agora - em <= TTL_META_MS => {}
                        _ => continue,
                    }
                }
                if let Ok(mut v) = serde_json::from_str(&d) {
                    if tipo == "event" {
                        expandir_evento(&mut v);
                    }
                    achados.insert(k, v);
                }
            }
        }
        Ok(achados)
    })
    .await
}

async fn meta_um(db: &Db, tipo: &'static str, chave: &str) -> Option<Value> {
    meta_varios(db, tipo, vec![chave.to_string()], true).await.ok()?.remove(chave)
}

/// `cacheSet()`: grava (ou atualiza) metadados com o instante atual.
async fn meta_gravar(db: &Db, tipo: &'static str, itens: Vec<(String, Value)>) {
    let itens: Vec<(String, Value)> = itens.into_iter().filter(|(_, v)| !v.is_null()).collect();
    if itens.is_empty() {
        return;
    }
    let _ = db
        .call(move |c| {
            let t = agora();
            let tx = c.transaction()?;
            {
                let mut s = tx.prepare(
                    "INSERT INTO kalshi_metadata_cache (kind, cache_key, data_json, updated_at) VALUES (?, ?, ?, ?)
        ON CONFLICT(kind, cache_key) DO UPDATE SET data_json=excluded.data_json, updated_at=excluded.updated_at",
                )?;
                for (k, v) in &itens {
                    let texto = if tipo == "event" { js::para_texto(&compactar_evento(v)) } else { js::para_texto(v) };
                    s.execute(rusqlite::params![tipo, k, texto, t])?;
                }
            }
            tx.commit()?;
            Ok(())
        })
        .await;
}

// ---------------------------------------------------------------------------
// Metadados públicos (sem credencial)
// ---------------------------------------------------------------------------

/// `kalshiMarket()`.
async fn mercado(db: &Db, ticker: &str) -> Option<Value> {
    if let Some(v) = meta_um(db, "market", ticker).await {
        return Some(v);
    }
    let t = net::codificar(ticker);
    for caminho in [format!("/trade-api/v2/markets/{t}"), format!("/trade-api/v2/historical/markets/{t}")] {
        if let Ok(Some(d)) = net::get_json(&format!("{BASE}{caminho}"), PRAZO_PUBLICO).await {
            let m = d.get("market").filter(|m| truthy(Some(m))).cloned().unwrap_or(d);
            meta_gravar(db, "market", vec![(ticker.to_string(), m.clone())]).await;
            return Some(m);
        }
    }
    None
}

/// `kalshiMarketsBatch()`: cache primeiro, depois lote atual e lote histórico.
async fn mercados_em_lote(db: &Db, tickers: &[String]) -> HashMap<String, Value> {
    let unicos: Vec<String> = {
        let mut vistos = HashSet::new();
        tickers.iter().filter(|t| !t.is_empty() && vistos.insert((*t).clone())).cloned().collect()
    };
    let mut achados = meta_varios(db, "market", unicos.clone(), true).await.unwrap_or_default();
    let faltantes: Vec<String> = unicos.iter().filter(|t| !achados.contains_key(*t)).cloned().collect();
    if faltantes.is_empty() {
        return achados;
    }
    for (rota, lista) in [("markets", faltantes.clone()), ("historical/markets", vec![])] {
        let lista = if rota == "markets" { lista } else { faltantes.iter().filter(|t| !achados.contains_key(*t)).cloned().collect() };
        if lista.is_empty() {
            continue;
        }
        let url = format!("{BASE}/trade-api/v2/{rota}?tickers={}&limit=1000", net::codificar(&lista.join(",")));
        if let Ok(Some(d)) = net::get_json(&url, PRAZO_PUBLICO).await {
            let mut novos = Vec::new();
            for m in d.get("markets").and_then(Value::as_array).map_or(&[][..], |a| a.as_slice()) {
                if let Some(t) = m.get("ticker").filter(|t| truthy(Some(t))) {
                    let t = js::string(Some(t));
                    novos.push((t.clone(), m.clone()));
                    achados.insert(t, m.clone());
                }
            }
            meta_gravar(db, "market", novos).await;
        }
    }
    achados
}

/// `kalshiStructuredTarget()`.
async fn alvo_estruturado(db: &Db, id: Option<&Value>) -> Option<Value> {
    if !truthy(id) {
        return None;
    }
    let id = js::string(id);
    if let Some(v) = meta_um(db, "target", &id).await {
        return Some(v);
    }
    let d = net::get_json(&format!("{BASE}/trade-api/v2/structured_targets/{}", net::codificar(&id)), PRAZO_PUBLICO).await.ok()??;
    let alvo = d.get("structured_target").filter(|v| truthy(Some(v))).cloned().unwrap_or(d);
    if !truthy(Some(&alvo)) {
        return None;
    }
    meta_gravar(db, "target", vec![(id, alvo.clone())]).await;
    Some(alvo)
}

fn evento_identificado(evento: Option<&Value>, ticker: &str) -> bool {
    evento.is_some_and(|e| {
        e.get("event_ticker").and_then(Value::as_str) == Some(ticker) && !texto(e.get("title")).trim().is_empty()
    })
}

/// `kalshiEvent()`: evento com milestones, mercados, confronto e datas oficiais.
async fn evento(db: &Db, ticker: &str) -> Option<Value> {
    let em_cache = meta_um(db, "event", ticker).await;
    if evento_identificado(em_cache.as_ref(), ticker) && kd::tem_milestone_esportivo(em_cache.as_ref()) {
        return em_cache;
    }
    let t = net::codificar(ticker);
    let url = format!("{BASE}/trade-api/v2/events?tickers={t}&with_milestones=true&with_nested_markets=true");
    // Falha de rede (ou JSON inválido) nesta primeira chamada anula o evento,
    // como o try/catch externo do Node; resposta não-2xx segue com `{}`.
    let dados = match net::get_json(&url, PRAZO_PUBLICO).await {
        Ok(d) => d.unwrap_or_else(|| json!({})),
        Err(_) => return None,
    };
    let candidato = match dados.get("events") {
        Some(Value::Array(lista)) => lista.iter().find(|e| e.get("event_ticker").and_then(Value::as_str) == Some(ticker)).cloned(),
        _ => Some(dados.get("event").filter(|e| truthy(Some(e))).cloned().unwrap_or_else(|| dados.clone())),
    };
    let mut ev: Map<String, Value> = candidato
        .filter(|c| c.get("event_ticker").and_then(Value::as_str) == Some(ticker))
        .and_then(|c| c.as_object().cloned())
        .unwrap_or_default();
    let mut milestones = dados.get("milestones").filter(|m| m.is_array()).cloned().unwrap_or_else(|| json!([]));
    let mut mercados = match ev.get("markets") {
        Some(m @ Value::Array(_)) => m.clone(),
        _ => dados.get("markets").filter(|m| m.is_array()).cloned().unwrap_or_else(|| json!([])),
    };
    // O Get Events pode omitir mercados históricos; o Get Event individual não.
    if let Ok(Some(d)) = net::get_json(&format!("{BASE}/trade-api/v2/events/{t}"), PRAZO_PUBLICO).await {
        let individual = d.get("event").filter(|e| truthy(Some(e))).cloned().unwrap_or(d);
        if individual.get("event_ticker").and_then(Value::as_str) == Some(ticker) {
            if let Some(o) = individual.as_object() {
                for (k, v) in o {
                    ev.insert(k.clone(), v.clone());
                }
            }
            if let Some(m @ Value::Array(_)) = individual.get("markets") {
                mercados = m.clone();
            }
        }
    }
    // Respostas vazias ou de outro evento não são metadado e não vão ao cache.
    if !evento_identificado(Some(&Value::Object(ev.clone())), ticker) {
        return None;
    }
    let url = format!("{BASE}/trade-api/v2/milestones?related_event_ticker={t}&limit=100");
    if let Ok(Some(d)) = net::get_json(&url, PRAZO_PUBLICO).await {
        if let Some(lista) = d.get("milestones").and_then(Value::as_array).filter(|l| !l.is_empty()) {
            milestones = Value::Array(lista.clone());
        }
    }
    let escolhido = kd::escolher_milestone(Some(&milestones), ticker);
    ev.insert("_milestones".into(), milestones);
    ev.insert("_milestone".into(), escolhido.clone());
    ev.insert("_markets".into(), mercados.clone());
    let detalhes = escolhido.get("details").filter(|d| truthy(Some(d))).cloned().unwrap_or_else(|| json!({}));
    let casa = ou(ou(detalhes.get("home_team_id"), detalhes.get("home_team")), detalhes.get("home_structured_target_id"));
    let fora = ou(ou(detalhes.get("away_team_id"), detalhes.get("away_structured_target_id")), detalhes.get("away_team"));
    if truthy(casa) || truthy(fora) {
        let (a, b) = futures::join!(alvo_estruturado(db, casa), alvo_estruturado(db, fora));
        let (nome_a, nome_b) = (texto(a.as_ref().and_then(|v| v.get("name"))), texto(b.as_ref().and_then(|v| v.get("name"))));
        if !nome_a.is_empty() && !nome_b.is_empty() {
            ev.insert("_teamsTitle".into(), Value::String(format!("{nome_a} vs {nome_b}")));
        }
    }
    if !truthy(ev.get("_teamsTitle")) {
        let mut textos: Vec<Option<&Value>> =
            vec![ev.get("title"), ev.get("sub_title"), ev.get("subtitle"), escolhido.get("title")];
        for m in mercados.as_array().map_or(&[][..], |a| a.as_slice()) {
            textos.extend([m.get("title"), m.get("subtitle"), m.get("yes_sub_title"), m.get("no_sub_title")]);
        }
        let confronto = kd::extrair_confronto(&textos);
        ev.insert("_teamsTitle".into(), Value::String(confronto));
    }
    // A data do milestone tem prioridade; as demais só preenchem lacunas.
    let ocorrencia = mercados
        .as_array()
        .and_then(|l| l.iter().find(|m| truthy(m.get("occurrence_datetime"))))
        .and_then(|m| m.get("occurrence_datetime").cloned())
        .unwrap_or(Value::Null);
    ev.insert("_occurrenceDate".into(), ocorrencia.clone());
    let inicio = [escolhido.get("start_date"), Some(&ocorrencia), ev.get("strike_date")]
        .into_iter()
        .find(|v| truthy(*v))
        .flatten()
        .cloned()
        .unwrap_or(Value::Null);
    ev.insert("_startDate".into(), inicio);
    let ev = Value::Object(ev);
    meta_gravar(db, "event", vec![(ticker.to_string(), ev.clone())]).await;
    Some(ev)
}

/// Mercados (lote + individuais) e eventos (6 em paralelo) para os tickers.
async fn metadados_para(db: &Db, tickers: &[String]) -> (HashMap<String, Value>, HashMap<String, Option<Value>>) {
    let mut mercados = mercados_em_lote(db, tickers).await;
    let unicos: Vec<String> = {
        let mut vistos = HashSet::new();
        tickers.iter().filter(|t| !t.is_empty() && vistos.insert((*t).clone())).cloned().collect()
    };
    for t in &unicos {
        if !mercados.contains_key(t) {
            if let Some(m) = mercado(db, t).await {
                mercados.insert(t.clone(), m);
            }
        }
    }
    let eventos_tickers: Vec<String> = {
        let mut vistos = HashSet::new();
        unicos
            .iter()
            .filter_map(|t| mercados.get(t).map(|m| texto(m.get("event_ticker"))))
            .filter(|e| !e.is_empty() && vistos.insert(e.clone()))
            .collect()
    };
    let mut eventos = HashMap::new();
    for lote in eventos_tickers.chunks(6) {
        let resultados = futures::future::join_all(lote.iter().map(|et| evento(db, et))).await;
        for (et, ev) in lote.iter().zip(resultados) {
            eventos.insert(et.clone(), ev);
        }
    }
    (mercados, eventos)
}

/// `atualizarModalidadesKalshiNoLedger()`, chamada pelo `GET /api/history/closed`.
///
/// Migração preguiçosa: só as 12 linhas Kalshi mais recentes que ainda guardam
/// a categoria raiz "Sports" consultam o evento (6 por vez). A modalidade vem
/// do milestone oficial do evento, nunca é deduzida sem fonte; linhas cujo
/// evento não pôde ser lido ficam para a próxima leitura do histórico.
pub async fn atualizar_modalidades_no_ledger(estado: &Estado) -> Result<()> {
    let user = estado.usuario.id.clone();
    let pendentes = estado.db.ler(move |c| history::pendentes_modalidade_kalshi(c, &user, 12)).await?;
    if pendentes.is_empty() {
        return Ok(());
    }
    let mut atualizadas = Vec::new();
    for lote in pendentes.chunks(6) {
        let eventos = futures::future::join_all(lote.iter().map(|linha| {
            let ticker = js::string(linha.get("_eventTicker"));
            async move { evento(&estado.db, &ticker).await }
        }))
        .await;
        for (linha, ev) in lote.iter().zip(eventos) {
            let Some(ev) = ev else { continue };
            // `{...row, ...metadadoEsporteKalshi(event)}`.
            let mut o = linha.as_object().cloned().unwrap_or_default();
            insere_esporte(&mut o, Some(&ev), None);
            if truthy(o.get("_sportMetaChecked")) {
                atualizadas.push(Value::Object(o));
            }
        }
    }
    if atualizadas.is_empty() {
        return Ok(());
    }
    let user = estado.usuario.id.clone();
    // O Node usa a gravação completa; a incremental lê do banco só as chaves
    // destas linhas e grava exatamente o mesmo resultado.
    estado.db.call(move |c| ledger::gravar(c, &user, atualizadas, true).map(|_| ())).await
}

fn insere_esporte(o: &mut Map<String, Value>, evento: Option<&Value>, mercado: Option<&Value>) {
    for (k, v) in kd::metadado_esporte(evento, mercado) {
        o.insert(k.into(), v);
    }
}

fn evento_de<'a>(eventos: &'a HashMap<String, Option<Value>>, m: &Value, linha: &Value) -> Option<&'a Value> {
    let chave = texto(ou(m.get("event_ticker"), linha.get("event_ticker")));
    eventos.get(&chave).and_then(Option::as_ref)
}

fn segundos_de(data: Option<&str>) -> f64 {
    data.and_then(js::data_ms).map_or(0.0, |ms| (ms / 1000.0).floor())
}

/// `enrichKalshi()`: posições abertas no formato das linhas do dashboard.
async fn enriquecer_posicoes(c: &Estado, linhas: Vec<Value>) -> Result<Vec<Value>> {
    let tickers: Vec<String> = linhas.iter().map(|p| texto(ou(p.get("ticker"), p.get("market_ticker")))).collect();
    let (mercados, eventos) = metadados_para(&c.db, &tickers).await;
    let vazio = json!({});
    let mut saida: Vec<Value> = linhas
        .iter()
        .map(|p| {
            let t = texto(ou(p.get("ticker"), p.get("market_ticker")));
            let m = mercados.get(&t).unwrap_or(&vazio);
            let ev = evento_de(&eventos, m, p);
            let posicao = js::number(Some(p.get("position_fp").filter(|v| truthy(Some(v))).unwrap_or(&json!(0))));
            let (qtd, sim) = (posicao.abs(), posicao > 0.0);
            let stake = js::number(Some(
                ou(p.get("market_exposure_dollars"), p.get("total_traded_dollars")).filter(|v| truthy(Some(v))).unwrap_or(&json!(0)),
            ));
            let inicio = kd::data_inicio(Some(m), ev, None);
            let ts = segundos_de(inicio.as_deref());
            // last_updated_ts da posição: instante do último fill, base da ordem "Negociado".
            let bruto = [p.get("last_updated_ts"), p.get("lastUpdatedTs"), p.get("last_updated_time")]
                .into_iter()
                .flatten()
                .find(|v| !v.is_null())
                .cloned()
                .unwrap_or(json!(0));
            let lido = match &bruto {
                Value::String(s) => js::data_ms(s).unwrap_or(f64::NAN),
                v => js::number(Some(v)),
            };
            let ultimo_ms = if lido.is_finite() && lido > 0.0 { if lido < 1e12 { lido * 1000.0 } else { lido } } else { 0.0 };
            let n = js::numero_json;
            let mut o = Map::new();
            o.insert("title".into(), json!(kd::kalshi_title(Some(m), ev, &t, Some(if sim { "yes" } else { "no" }))));
            o.insert("outcome".into(), json!(if sim { "Yes" } else { "No" }));
            o.insert("avgPrice".into(), if qtd != 0.0 && !qtd.is_nan() { n(stake / qtd) } else { Value::Null });
            o.insert("initialValue".into(), n(stake));
            o.insert("totalBought".into(), n(qtd));
            o.insert("size".into(), n(qtd));
            o.insert("cashPnl".into(), n(js::number(Some(p.get("realized_pnl_dollars").filter(|v| truthy(Some(v))).unwrap_or(&json!(0))))));
            o.insert("conditionId".into(), json!(format!("kalshi:{t}")));
            o.insert("_platform".into(), json!("kalshi"));
            o.insert("_eventTicker".into(), ou(m.get("event_ticker"), p.get("event_ticker")).filter(|v| truthy(Some(v))).cloned().unwrap_or(Value::Null));
            o.insert("_matchupKey".into(), ev.and_then(|e| e.get("_teamsTitle")).filter(|v| truthy(Some(v))).cloned().unwrap_or(json!("")));
            o.insert("timestamp".into(), n(ts));
            o.insert("_lastTradeTs".into(), n(if ultimo_ms != 0.0 { (ultimo_ms / 1000.0).floor() } else { 0.0 }));
            o.insert("_startDate".into(), inicio.map_or(Value::Null, Value::String));
            o.insert(
                "endDate".into(),
                [m.get("close_time"), m.get("expiration_time"), ev.and_then(|e| e.get("close_time"))]
                    .into_iter()
                    .find(|v| truthy(*v))
                    .flatten()
                    .cloned()
                    .unwrap_or(Value::Null),
            );
            o.insert("eventSlug".into(), m.get("event_ticker").filter(|v| truthy(Some(v))).cloned().unwrap_or_else(|| json!(t)));
            o.insert("slug".into(), json!(t));
            o.insert("seriesTicker".into(), m.get("series_ticker").filter(|v| truthy(Some(v))).cloned().unwrap_or(Value::Null));
            o.insert("_kalshiTitleFormatVersion".into(), json!(kd::VERSAO_TITULO));
            insere_esporte(&mut o, ev, Some(m));
            Value::Object(o)
        })
        .collect();
    // Só títulos sem confronto precisam dos metadados persistidos.
    let pendentes: Vec<usize> = (0..saida.len()).filter(|&i| titles::kalshi_title_missing_matchup(&saida[i], None)).collect();
    if !pendentes.is_empty() {
        let linhas_raw: Vec<history::Linha> =
            pendentes.iter().map(|&i| serde_json::from_str(&js::para_texto(&saida[i]))).collect::<Result<_, _>>()?;
        let recuperadas = c.db.ler(move |conn| history::anexar_metadados_kalshi(conn, linhas_raw)).await?;
        let por_slug: HashMap<String, Value> = recuperadas
            .iter()
            .filter_map(|l| serde_json::from_str::<Value>(&js::para_texto(l)).ok())
            .map(|v| (texto(v.get("slug")), v))
            .collect();
        for linha in &mut saida {
            if let Some(v) = por_slug.get(&texto(linha.get("slug"))) {
                *linha = v.clone();
            }
        }
    }
    Ok(saida)
}

/// `enrichSettlements()`: liquidações no formato do ledger.
async fn enriquecer_liquidacoes(c: &Estado, linhas: &[Value]) -> Result<Vec<Value>> {
    let tickers: Vec<String> = linhas.iter().map(|s| texto(ou(s.get("ticker"), s.get("market_ticker")))).collect();
    let (mercados, eventos) = metadados_para(&c.db, &tickers).await;
    let vazio = json!({});
    let mut saida = Vec::with_capacity(linhas.len());
    for s in linhas {
        for k in ["yes_count_fp", "no_count_fp", "yes_total_cost_dollars", "no_total_cost_dollars", "fee_cost", "revenue"] {
            let v = s.get(k);
            let n = js::number(v);
            if v.is_none_or(Value::is_null) || v == Some(&json!("")) || !n.is_finite() || n < 0.0 {
                bail!("Liquidação Kalshi sem valores financeiros completos.");
            }
        }
        let t = texto(ou(s.get("ticker"), s.get("market_ticker")));
        let m = mercados.get(&t).unwrap_or(&vazio);
        let ev = evento_de(&eventos, m, s);
        let num = |k: &str| js::number(Some(s.get(k).filter(|v| truthy(Some(v))).unwrap_or(&json!(0))));
        let (sim, nao) = (num("yes_count_fp"), num("no_count_fp"));
        let contratos = sim + nao;
        let custo = num("yes_total_cost_dollars") + num("no_total_cost_dollars");
        let taxa = num("fee_cost");
        let stake = custo + taxa;
        let pagamento = num("revenue") / 100.0;
        let pnl = pagamento - stake;
        let liquidada = s.get("settled_time");
        let inicio = kd::data_inicio(Some(m), ev, liquidada);
        let ts = segundos_de(inicio.as_deref());
        let (so_sim, so_nao) = (sim != 0.0 && nao == 0.0, nao != 0.0 && sim == 0.0);
        let lado = if so_sim { Some("yes") } else if so_nao { Some("no") } else { None };
        let n = js::numero_json;
        let contratos_ou_1 = if contratos != 0.0 && !contratos.is_nan() { contratos } else { 1.0 };
        let mut o = Map::new();
        o.insert("title".into(), json!(kd::kalshi_title(Some(m), ev, &t, lado)));
        o.insert(
            "outcome".into(),
            json!(if so_sim {
                "Yes".to_string()
            } else if so_nao {
                "No".to_string()
            } else {
                let r = s.get("market_result");
                if truthy(r) { js::string(r) } else { "—".into() }
            }),
        );
        o.insert("avgPrice".into(), if contratos != 0.0 && !contratos.is_nan() { n(custo / contratos) } else { Value::Null });
        o.insert("initialValue".into(), n(stake));
        o.insert("totalBought".into(), n(contratos_ou_1));
        o.insert("size".into(), n(contratos_ou_1));
        o.insert("realizedPnl".into(), n(pnl));
        o.insert("_feeCost".into(), n(taxa));
        o.insert("_grossCost".into(), n(custo));
        o.insert("_payout".into(), n(pagamento));
        o.insert("conditionId".into(), json!(format!("kalshi-settled:{t}")));
        o.insert("_platform".into(), json!("kalshi"));
        o.insert("_eventTicker".into(), ou(m.get("event_ticker"), s.get("event_ticker")).filter(|v| truthy(Some(v))).cloned().unwrap_or(Value::Null));
        o.insert("_matchupKey".into(), ev.and_then(|e| e.get("_teamsTitle")).filter(|v| truthy(Some(v))).cloned().unwrap_or(json!("")));
        o.insert("timestamp".into(), n(ts));
        o.insert("_startDate".into(), inicio.map_or(Value::Null, Value::String));
        o.insert("_settledTime".into(), liquidada.filter(|v| truthy(Some(v))).cloned().unwrap_or(Value::Null));
        o.insert("eventSlug".into(), m.get("event_ticker").filter(|v| truthy(Some(v))).cloned().unwrap_or_else(|| json!(t)));
        o.insert("slug".into(), json!(t));
        o.insert("seriesTicker".into(), m.get("series_ticker").filter(|v| truthy(Some(v))).cloned().unwrap_or(Value::Null));
        o.insert("_kalshiTitleFormatVersion".into(), json!(kd::VERSAO_TITULO));
        insere_esporte(&mut o, ev, Some(m));
        saida.push(Value::Object(o));
    }
    Ok(saida)
}

async fn carregar_liquidacoes(c: &Estado) -> Result<HashMap<String, Liquidacao>> {
    let user = c.usuario.id.clone();
    if let Some(m) = c.kalshi.liquidacoes.lock().map_err(|_| anyhow!("cache indisponível"))?.get(&user) {
        return Ok(m.clone());
    }
    let u = user.clone();
    let mapa = c
        .db
        .ler(move |conn| {
            let mut s = conn.prepare("SELECT settlement_key,source_signature,data_json,updated_at FROM kalshi_settlement_cache WHERE user_id=?")?;
            let mut mapa = HashMap::new();
            for r in s.query_map([&u], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?)))? {
                let (k, a, d, em) = r?;
                if let Ok(linha) = serde_json::from_str(&d) {
                    mapa.insert(k, Liquidacao { assinatura: a, linha, em: js::data_ms(&em).unwrap_or(0.0) });
                }
            }
            Ok(mapa)
        })
        .await?;
    if let Ok(mut m) = c.kalshi.liquidacoes.lock() {
        m.insert(user, mapa.clone());
    }
    Ok(mapa)
}

async fn salvar_liquidacoes(c: &Estado, itens: Vec<(String, String, Value)>) -> Result<Vec<(String, Liquidacao)>> {
    let user = c.usuario.id.clone();
    let gravados = c
        .db
        .call(move |conn| {
            let t = agora();
            let em = js::data_ms(&t).unwrap_or(0.0);
            let tx = conn.transaction()?;
            let mut saida = Vec::new();
            {
                let mut s = tx.prepare(
                    "INSERT INTO kalshi_settlement_cache(user_id,settlement_key,source_signature,data_json,updated_at) VALUES(?,?,?,?,?) ON CONFLICT(user_id,settlement_key) DO UPDATE SET source_signature=excluded.source_signature,data_json=excluded.data_json,updated_at=excluded.updated_at",
                )?;
                for (k, a, linha) in itens {
                    s.execute(rusqlite::params![user, k, a, js::para_texto(&linha), t])?;
                    saida.push((k, Liquidacao { assinatura: a, linha, em }));
                }
            }
            tx.commit()?;
            Ok(saida)
        })
        .await?;
    if let Ok(mut m) = c.kalshi.liquidacoes.lock() {
        let mapa = m.entry(c.usuario.id.clone()).or_default();
        for (k, l) in &gravados {
            mapa.insert(k.clone(), l.clone());
        }
    }
    Ok(gravados)
}

/// `lerMetadadosKalshiPersistidos()` (sem TTL): metadados imutáveis de liquidados.
async fn metadados_persistidos(c: &Estado, tipo: &'static str, chaves: Vec<String>) -> Result<HashMap<String, Value>> {
    let chaves: Vec<String> = chaves.into_iter().map(|k| k.trim().to_string()).filter(|k| !k.is_empty()).collect();
    meta_varios(&c.db, tipo, chaves, false).await
}

/// `corrigirTitulosKalshiCacheados()`: refaz títulos de versões antigas que
/// eram perguntas binárias, sem reconsultar a API.
async fn corrigir_titulos_cacheados(c: &Estado, linhas: &[Value], cache: &HashMap<String, Liquidacao>) -> Result<Vec<(String, String, Value)>> {
    let user = c.usuario.id.clone();
    let revisados = c.kalshi.revisados.lock().map_err(|_| anyhow!("cache indisponível"))?.get(&user).cloned().unwrap_or_default();
    let mut candidatos = Vec::new();
    for fonte in linhas {
        let chave = kd::chave_liquidacao(fonte);
        let assinatura = kd::assinatura_liquidacao(fonte);
        let Some(em_cache) = cache.get(&chave) else { continue };
        if em_cache.assinatura != assinatura || !kd::titulo_precisa_revisar(&em_cache.linha) || revisados.get(&chave) == Some(&assinatura) {
            continue;
        }
        let ticker = texto(ou(ou(em_cache.linha.get("slug"), em_cache.linha.get("ticker")), em_cache.linha.get("market_ticker"))).trim().to_string();
        if !ticker.is_empty() {
            candidatos.push((chave, assinatura, em_cache.linha.clone(), ticker));
        }
    }
    if candidatos.is_empty() {
        return Ok(vec![]);
    }
    let mercados = metadados_persistidos(c, "market", candidatos.iter().map(|c| c.3.clone()).collect()).await?;
    let eventos = metadados_persistidos(c, "event", mercados.values().map(|m| texto(m.get("event_ticker"))).collect()).await?;
    let mut corrigidos = Vec::new();
    let mut novos_revisados = Vec::new();
    for (chave, assinatura, linha, ticker) in candidatos {
        let Some(m) = mercados.get(&ticker) else { continue };
        let Some(ev) = m.get("event_ticker").and_then(Value::as_str).and_then(|e| eventos.get(e)) else { continue };
        novos_revisados.push((chave.clone(), assinatura.clone()));
        if kd::pergunta_binaria(Some(m)).is_empty() {
            continue;
        }
        let resultado = texto(linha.get("outcome")).to_lowercase();
        let lado = match resultado.as_str() {
            "yes" => Some("yes"),
            "no" => Some("no"),
            _ => None,
        };
        let titulo = kd::kalshi_title(Some(m), Some(ev), &ticker, lado);
        if titulo.is_empty() || Some(titulo.as_str()) == linha.get("title").and_then(Value::as_str) {
            continue;
        }
        let mut o = linha.as_object().cloned().unwrap_or_default();
        o.insert("title".into(), json!(titulo));
        o.insert("_kalshiTitleFormatVersion".into(), json!(kd::VERSAO_TITULO));
        corrigidos.push((chave, assinatura, Value::Object(o)));
    }
    if let Ok(mut m) = c.kalshi.revisados.lock() {
        let mapa = m.entry(user).or_default();
        for (k, a) in novos_revisados {
            mapa.insert(k, a);
        }
    }
    Ok(corrigidos)
}

/// `enrichSettlementsIncremental()`: só enriquece liquidações novas, alteradas
/// ou incompletas há mais de 10 minutos.
async fn liquidacoes_incrementais(c: &Estado, linhas: &[Value]) -> Result<Vec<Value>> {
    let mut cache = carregar_liquidacoes(c).await?;
    let corrigidos = corrigir_titulos_cacheados(c, linhas, &cache).await?;
    for (k, l) in salvar_liquidacoes(c, corrigidos).await? {
        cache.insert(k, l);
    }
    let agora = agora_ms();
    let pendentes: Vec<&Value> = linhas
        .iter()
        .filter(|s| match cache.get(&kd::chave_liquidacao(s)) {
            None => true,
            Some(l) => {
                l.assinatura != kd::assinatura_liquidacao(s)
                    || kd::data_inicio_invalida(Some(&l.linha))
                    || (kd::liquidacao_incompleta(Some(&l.linha)) && agora - l.em >= REPETIR_INCOMPLETA_MS)
            }
        })
        .collect();
    if !pendentes.is_empty() {
        let fontes: Vec<Value> = pendentes.iter().map(|v| (*v).clone()).collect();
        let enriquecidas = enriquecer_liquidacoes(c, &fontes).await?;
        let itens = fontes
            .iter()
            .zip(enriquecidas)
            .map(|(s, linha)| (kd::chave_liquidacao(s), kd::assinatura_liquidacao(s), linha))
            .collect();
        for (k, l) in salvar_liquidacoes(c, itens).await? {
            cache.insert(k, l);
        }
    }
    Ok(linhas.iter().filter_map(|s| cache.get(&kd::chave_liquidacao(s)).map(|l| l.linha.clone())).collect())
}

// ---------------------------------------------------------------------------
// Chamadas privadas
// ---------------------------------------------------------------------------

/// Credenciais decifradas só durante a chamada.
async fn credenciais(c: &Estado) -> Result<(zeroize::Zeroizing<String>, zeroize::Zeroizing<String>)> {
    let user = c.usuario.id.clone();
    let linha: Option<(Option<String>, Option<String>)> = c
        .db
        .ler(move |conn| {
            Ok(conn
                .query_row("SELECT kalshi_api_key_enc,kalshi_private_key_enc FROM connections WHERE user_id=?", [&user], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .optional()?)
        })
        .await?;
    match linha {
        Some((Some(k), Some(p))) if !k.is_empty() && !p.is_empty() => Ok((c.chave.decrypt(&k)?, c.chave.decrypt(&p)?)),
        _ => bail!("Conecte sua conta Kalshi primeiro."),
    }
}

/// `kalshiSignedFetch()` + leitura: `Err(ErroHttp)` para respostas não-2xx.
async fn privada(c: &Estado, caminho: &str) -> Result<Value> {
    let (chave_api, pem) = credenciais(c).await?;
    let ts = chrono::Utc::now().timestamp_millis().to_string();
    let so_caminho = caminho.split('?').next().unwrap_or(caminho);
    let assinatura = {
        let par = net::chave_rsa(&pem)?;
        net::assinar_pss(&par, format!("{ts}GET{so_caminho}").as_bytes())?
    };
    let r = net::CLIENTE
        .get(format!("{BASE}{caminho}"))
        .header("KALSHI-ACCESS-KEY", chave_api.as_str())
        .header("KALSHI-ACCESS-SIGNATURE", assinatura)
        .header("KALSHI-ACCESS-TIMESTAMP", ts)
        .header("accept", "application/json")
        .timeout(PRAZO_PRIVADO)
        .send()
        .await?;
    let status = r.status();
    let corpo = r.text().await?;
    let dados: Value = serde_json::from_str(&corpo).unwrap_or_else(|_| json!({}));
    if !status.is_success() {
        return Err(ErroHttp { status: status.as_u16(), corpo: dados }.into());
    }
    Ok(dados)
}

/// `kalshiPaginate()` + `collectPages()`: percorre até o fim, sem cursor repetido.
async fn paginar(c: &Estado, base: &str, campo: &str) -> Result<Vec<Value>> {
    let mut saida = Vec::new();
    let mut vistos = HashSet::new();
    let mut cursor = String::new();
    for _ in 0..1000 {
        let sep = if base.contains('?') { '&' } else { '?' };
        let caminho = format!(
            "{base}{sep}limit=1000{}",
            if cursor.is_empty() { String::new() } else { format!("&cursor={}", net::codificar(&cursor)) }
        );
        let dados = privada(c, &caminho).await?;
        let Some(lista) = dados.get(campo).and_then(Value::as_array) else { bail!("Resposta de histórico inválida") };
        saida.extend(lista.iter().cloned());
        cursor = texto(dados.get("cursor"));
        if cursor.is_empty() {
            return Ok(saida);
        }
        if !vistos.insert(cursor.clone()) {
            bail!("Histórico incompleto: cursor repetido");
        }
    }
    bail!("Histórico incompleto: limite de paginação atingido")
}

// ---------------------------------------------------------------------------
// Rotas
// ---------------------------------------------------------------------------

fn resposta(status: StatusCode, body: Value) -> ApiResponse {
    ApiResponse { status, body, bruto: None }
}

fn status_ou_502(e: &anyhow::Error) -> StatusCode {
    net::status_do_erro(e).and_then(|s| StatusCode::from_u16(s).ok()).unwrap_or(StatusCode::BAD_GATEWAY)
}

fn saldo(d: &Value) -> Value {
    let n = |k: &str| js::number(Some(d.get(k).filter(|v| truthy(Some(v))).unwrap_or(&json!(0)))) / 100.0;
    json!({ "ok": true, "balance": js::numero_json(n("balance")), "portfolioValue": js::numero_json(n("portfolio_value")) })
}

/// `GET /api/kalshi/test`.
pub async fn testar(c: &Estado) -> ApiResponse {
    match privada(c, "/trade-api/v2/portfolio/balance").await {
        Ok(d) => resposta(StatusCode::OK, saldo(&d)),
        Err(e) => {
            let status = status_ou_502(&e);
            let msg = if status == StatusCode::UNAUTHORIZED {
                "Credenciais Kalshi inválidas ou sem permissão de leitura."
            } else {
                "Não foi possível conectar à Kalshi."
            };
            resposta(status, json!({ "ok": false, "error": msg }))
        }
    }
}

/// `GET /api/kalshi/balance` (`kalshiPrivate()`).
pub async fn saldo_rota(c: &Estado) -> ApiResponse {
    if credenciais(c).await.is_err() {
        return resposta(StatusCode::CONFLICT, json!({ "ok": false, "error": "Conecte sua conta Kalshi primeiro." }));
    }
    match privada(c, "/trade-api/v2/portfolio/balance").await {
        Ok(d) => resposta(StatusCode::OK, saldo(&d)),
        Err(e) => match e.downcast_ref::<ErroHttp>() {
            Some(h) => resposta(
                StatusCode::from_u16(h.status).unwrap_or(StatusCode::BAD_GATEWAY),
                json!({ "ok": false, "error": format!("Kalshi {}", h.status), "details": h.corpo }),
            ),
            None => {
                eprintln!("[kalshi] saldo: {e:#}");
                resposta(StatusCode::BAD_GATEWAY, json!({ "ok": false, "error": "Falha ao consultar Kalshi." }))
            }
        },
    }
}

/// `GET /api/kalshi/positions`.
pub async fn posicoes(c: &Estado) -> ApiResponse {
    let chave = format!("{}|positions", c.usuario.id);
    if let Some(v) = c.kalshi.endpoint_get(&chave) {
        return resposta(StatusCode::OK, v);
    }
    let resultado: Result<Value> = async {
        let linhas: Vec<Value> = paginar(c, "/trade-api/v2/portfolio/positions", "market_positions")
            .await?
            .into_iter()
            .filter(|p| js::number(Some(p.get("position_fp").filter(|v| truthy(Some(v))).unwrap_or(&json!(0)))) != 0.0)
            .collect();
        Ok(json!({ "ok": true, "abertos": enriquecer_posicoes(c, linhas).await? }))
    }
    .await;
    match resultado {
        Ok(corpo) => {
            c.kalshi.endpoint_set(chave, corpo.clone());
            resposta(StatusCode::OK, corpo)
        }
        Err(e) => {
            eprintln!("[kalshi] posições: {e:#}");
            resposta(status_ou_502(&e), json!({ "ok": false, "error": "Falha ao consultar posições da Kalshi." }))
        }
    }
}

/// `GET /api/kalshi/settlements`: pagina até o fim, inclusive na primeira sincronização.
pub async fn liquidacoes(c: &Estado) -> ApiResponse {
    let chave = format!("{}|settlements", c.usuario.id);
    if let Some(v) = c.kalshi.endpoint_get(&chave) {
        return resposta(StatusCode::OK, v);
    }
    let resultado: Result<Value> = async {
        let linhas = paginar(c, "/trade-api/v2/portfolio/settlements", "settlements").await?;
        Ok(json!({ "ok": true, "fechados": liquidacoes_incrementais(c, &linhas).await? }))
    }
    .await;
    match resultado {
        Ok(corpo) => {
            c.kalshi.endpoint_set(chave, corpo.clone());
            resposta(StatusCode::OK, corpo)
        }
        Err(e) => {
            eprintln!("[kalshi] liquidações: {e:#}");
            resposta(status_ou_502(&e), json!({ "ok": false, "error": "Falha ao consultar histórico da Kalshi." }))
        }
    }
}

/// `clearKalshiUserCaches()`.
async fn limpar_caches(c: &Estado) -> Result<()> {
    c.kalshi.limpar_usuario(&c.usuario.id);
    let user = c.usuario.id.clone();
    c.db.call(move |conn| {
        conn.execute("DELETE FROM kalshi_settlement_cache WHERE user_id=?", [&user])?;
        Ok(())
    })
    .await
}

/// `POST /api/connections/kalshi`: valida e cifra a API Key e a Private Key.
pub async fn conectar(c: &Estado, corpo: &Value) -> Result<ApiResponse> {
    let chave = texto(corpo.get("apiKey")).trim().to_string();
    let privada_pem = zeroize::Zeroizing::new(texto(corpo.get("privateKey")).trim().to_string());
    if chave.is_empty() || !privada_pem.contains("PRIVATE KEY") {
        return Ok(resposta(StatusCode::BAD_REQUEST, json!({ "ok": false, "error": "Informe a API Key e a Private Key da Kalshi." })));
    }
    if net::chave_rsa(&privada_pem).is_err() {
        return Ok(resposta(StatusCode::BAD_REQUEST, json!({ "ok": false, "error": "A Private Key da Kalshi não parece ser um PEM válido." })));
    }
    let (k, p) = match (c.chave.encrypt(&chave), c.chave.encrypt(&privada_pem)) {
        (Ok(k), Ok(p)) => (k, p),
        _ => return Ok(resposta(StatusCode::INTERNAL_SERVER_ERROR, json!({ "ok": false, "error": "Não foi possível proteger as credenciais da Kalshi." }))),
    };
    let user = c.usuario.id.clone();
    c.db.call(move |conn| {
        conn.execute(
            "INSERT INTO connections(user_id,kalshi_api_key_enc,kalshi_private_key_enc,updated_at) VALUES(?,?,?,?) ON CONFLICT(user_id) DO UPDATE SET kalshi_api_key_enc=excluded.kalshi_api_key_enc,kalshi_private_key_enc=excluded.kalshi_private_key_enc,updated_at=excluded.updated_at",
            rusqlite::params![user, k, p, agora()],
        )?;
        Ok(())
    })
    .await?;
    limpar_caches(c).await?;
    Ok(resposta(StatusCode::OK, json!({ "ok": true })))
}

/// `DELETE /api/connections/kalshi`.
pub async fn desconectar(c: &Estado) -> Result<ApiResponse> {
    let user = c.usuario.id.clone();
    c.db.call(move |conn| {
        conn.execute(
            "UPDATE connections SET kalshi_api_key_enc=NULL,kalshi_private_key_enc=NULL,updated_at=? WHERE user_id=?",
            rusqlite::params![agora(), user],
        )?;
        Ok(())
    })
    .await?;
    limpar_caches(c).await?;
    Ok(resposta(StatusCode::OK, json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evento() -> Value {
        json!({
            "event_ticker": "EV-1", "title": "A vs B",
            "markets": [{ "ticker": "M-1", "yes_sub_title": "A", "volume": 1.50 }, { "ticker": "M-2", "z": { "b": 1, "a": 2 } }],
            "product_metadata": { "competition_scope": "Liga" },
            "_milestones": [{ "id": "x", "type": "soccer_tournament_multi_leg" }, { "id": "y", "type": "football_game", "start_date": "2026-09-20" }],
            "_milestone": { "id": "y", "type": "football_game", "start_date": "2026-09-20" },
            "_markets": [{ "ticker": "M-1", "yes_sub_title": "A", "volume": 1.50 }, { "ticker": "M-2", "z": { "b": 1, "a": 2 } }],
            "_teamsTitle": "A vs B", "_occurrenceDate": null, "_startDate": "2026-09-20"
        })
    }

    fn ida_e_volta(ev: &Value) -> (String, String) {
        let gravado = js::para_texto(&compactar_evento(ev));
        let mut lido: Value = serde_json::from_str(&gravado).unwrap();
        expandir_evento(&mut lido);
        (gravado, js::para_texto(&lido))
    }

    #[test]
    fn evento_compactado_volta_igual() {
        let ev = evento();
        let (gravado, lido) = ida_e_volta(&ev);
        assert_eq!(lido, js::para_texto(&ev), "a leitura devolve o evento como era");
        assert!(gravado.len() < lido.len(), "as cópias saem do texto gravado");
        let o: Value = serde_json::from_str(&gravado).unwrap();
        assert_eq!(o["markets"], json!(format!("{COPIA}_markets")));
        assert_eq!(o["_milestone"], json!(format!("{COPIA}_milestones/1")));
        let chaves: Vec<&String> = o.as_object().unwrap().keys().collect();
        let originais: Vec<&String> = ev.as_object().unwrap().keys().collect();
        assert_eq!(chaves, originais, "as chaves ficam na mesma ordem");

        // Sem cópia de verdade, nada muda.
        let mut v = evento();
        v["_markets"][1]["z"] = json!({ "a": 2, "b": 1 });
        v["_milestone"] = json!({ "id": "outro" });
        assert_eq!(ida_e_volta(&v).0, js::para_texto(&v), "mesmo conteúdo em outra ordem não é cópia");
        for remover in ["markets", "_markets", "_milestones", "_milestone"] {
            let mut v = evento();
            v.as_object_mut().unwrap().remove(remover);
            let (_, lido) = ida_e_volta(&v);
            assert_eq!(lido, js::para_texto(&v), "sem {remover}");
        }
        let mut v = evento();
        v["_milestones"] = json!("não é lista");
        assert_eq!(ida_e_volta(&v).1, js::para_texto(&v));
        assert_eq!(ida_e_volta(&json!(["não é objeto"])).1, "[\"não é objeto\"]");
    }

    #[test]
    fn cache_de_eventos_compacta_uma_vez_e_le_igual() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = crate::db::abrir(&dir.path().join("t.db")).unwrap();
        // Um evento gravado antes da compactação e um mercado (que fica como está).
        c.execute(
            "INSERT INTO kalshi_metadata_cache(kind,cache_key,data_json,updated_at) VALUES('event','EV-1',?,'2026-01-01T00:00:00.000Z'),('market','M-1',?,'x')",
            rusqlite::params![js::para_texto(&evento()), js::para_texto(&evento()["markets"][0])],
        )
        .unwrap();
        let (mudados, antes, depois) = compactar_cache_uma_vez(&mut c).unwrap().unwrap();
        assert_eq!(mudados, 1);
        assert!(depois < antes);
        assert!(compactar_cache_uma_vez(&mut c).unwrap().is_none(), "roda uma vez só");
        let em: String = c.query_row("SELECT updated_at FROM kalshi_metadata_cache WHERE cache_key='EV-1'", [], |r| r.get(0)).unwrap();
        assert_eq!(em, "2026-01-01T00:00:00.000Z", "o prazo de validade não muda");
        let db = crate::db::Db::com_leitura(c, 1).unwrap();
        let lidos = tauri::async_runtime::block_on(meta_varios(&db, "event", vec!["EV-1".into()], false)).unwrap();
        assert_eq!(js::para_texto(&lidos["EV-1"]), js::para_texto(&evento()));
        tauri::async_runtime::block_on(meta_gravar(&db, "event", vec![("EV-2".into(), evento())]));
        let lidos = tauri::async_runtime::block_on(meta_varios(&db, "event", vec!["EV-2".into()], true)).unwrap();
        assert_eq!(js::para_texto(&lidos["EV-2"]), js::para_texto(&evento()), "gravado agora e lido de volta");
    }

    /// Num banco real (cópia em BETTY_COMPARAR_DB): cada evento do cache,
    /// compactado e lido de volta, dá o mesmo texto de antes.
    #[test]
    #[ignore]
    fn oraculo_cache_de_eventos() {
        let c = crate::db::abrir(std::path::Path::new(&std::env::var("BETTY_COMPARAR_DB").unwrap())).unwrap();
        let mut s = c.prepare("SELECT cache_key,data_json FROM kalshi_metadata_cache WHERE kind='event'").unwrap();
        let (mut n, mut antes, mut depois, mut com_markets, mut com_milestone) = (0, 0, 0, 0, 0);
        for linha in s.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).unwrap() {
            let (chave, texto) = linha.unwrap();
            let ev: Value = serde_json::from_str(&texto).unwrap();
            let (gravado, lido) = ida_e_volta(&ev);
            assert_eq!(lido, js::para_texto(&ev), "{chave}");
            n += 1;
            antes += texto.len();
            depois += gravado.len();
            com_markets += usize::from(gravado.contains(&format!("\"markets\":\"{}", "\\u0000betty:")));
            com_milestone += usize::from(gravado.contains("\"_milestone\":\"\\u0000betty:"));
        }
        eprintln!("{n} eventos iguais na ida e volta | {:.1} MB -> {:.1} MB | markets: {com_markets}, _milestone: {com_milestone}", antes as f64 / 1e6, depois as f64 / 1e6);
    }
}