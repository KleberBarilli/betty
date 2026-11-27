//! Escrita do histórico fechado, igual ao `server.js`: o ledger é a memória
//! permanente da aposta; o merge preserva identidade e datas confirmadas e
//! atualiza só os valores financeiros. Uma linha só é regravada quando o JSON
//! resultante muda como texto.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use rusqlite::types::ToSql;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Map, Value, json};

use crate::api::CASAS;
use crate::audit;
use crate::db::agora;
use crate::history::{self, Linha, ymd_sao_paulo};
use crate::js;
use crate::regex_js;
use crate::titles;

/// `casaHistorico()`.
pub fn casa_historico(v: Option<&Value>) -> &'static str {
    let casa = if js::truthy(v) { js::string(v).to_lowercase() } else { "polymarket".into() };
    CASAS.iter().copied().find(|c| *c == casa).unwrap_or("polymarket")
}

fn texto_truthy(v: Option<&Value>) -> String {
    if js::truthy(v) { js::string(v) } else { String::new() }
}

/// `closedLedgerKey()`: chave canônica da aposta no ledger.
pub fn chave_ledger(d: &Value) -> String {
    chave_ledger_casa(d, casa_historico(d.get("_platform")))
}

/// `closedLedgerKey()` com a casa já resolvida.
fn chave_ledger_casa(d: &Value, casa: &str) -> String {
    if casa != "polymarket" {
        let v = ["slug", "ticker", "market_ticker", "conditionId"].iter().map(|k| d.get(*k)).find(|v| js::truthy(*v)).flatten();
        return texto_truthy(v).trim().to_string();
    }
    let condicao = texto_truthy(d.get("conditionId")).trim().to_string();
    let asset = texto_truthy(d.get("asset")).trim().to_string();
    // `outcomeIndex ?? outcome ?? ''`: só null/undefined passam para o próximo.
    let resultado = [d.get("outcomeIndex"), d.get("outcome")].into_iter().flatten().find(|v| !v.is_null());
    let resultado = resultado.map_or(String::new(), |v| js::string(Some(v))).trim().to_string();
    format!("{condicao}|{asset}|{resultado}")
}

fn chave_valida(chave: &str) -> bool {
    !chave.is_empty() && !chave.replace('|', "").is_empty()
}

regex_js!(TF_FINAL_VENCEDOR, r"(?i)\b(?:winner\??|to\s+advance|advances?)\s*$");
regex_js!(TF_VS, r"(?i)\bvs\.?\b|\bversus\b|\b@\b");
regex_js!(TF_GOLS, r"(?i)\bwill\s+(?:over|under)\b.*\bgoals?\b");
regex_js!(TF_REG_TIME, r"(?i)\breg(?:ulation)?\s*time\b.*\bto\s+win\b");
regex_js!(TF_INICIO, r"(?i)^(will|over|under|yes|no|winner|total|moneyline)\b");

/// `tituloFracoLedger()`: título que é só o texto bruto do mercado.
pub fn titulo_fraco(v: Option<&Value>) -> bool {
    let s = texto_truthy(v);
    let s = s.trim();
    if s.is_empty() {
        return true;
    }
    let vs = TF_VS.is_match(s);
    if (TF_FINAL_VENCEDOR.is_match(s) || TF_GOLS.is_match(s) || TF_REG_TIME.is_match(s)) && vs {
        return true;
    }
    TF_INICIO.is_match(s) && !vs
}

/// Atribuição do JS: `undefined` some do JSON; qualquer outro valor fica.
fn atribuir(m: &mut Map<String, Value>, chave: &str, valor: Option<&Value>) {
    match valor {
        Some(v) => {
            m.insert(chave.into(), v.clone());
        }
        None => {
            m.shift_remove(chave);
        }
    }
}

fn e_numero_finito(v: Option<&Value>) -> bool {
    v.and_then(Value::as_number).and_then(|n| n.as_f64()).is_some_and(f64::is_finite)
}

/// `ledgerMerge()`. `Err` quando o JS lançaria exceção.
pub fn merge(antiga: &Value, nova: &Value) -> Result<Value, ()> {
    let mut m = Map::new();
    // `{...(newRow||{}), ...(oldRow||{})}`
    for fonte in [nova, antiga] {
        if let Some(o) = fonte.as_object() {
            for (k, v) in o {
                m.insert(k.clone(), v.clone());
            }
        }
    }
    let g = |v: &Value, k: &str| v.get(k).cloned();
    if nova.get("_platform").and_then(Value::as_str) == Some("predict") {
        if js::truthy(nova.get("_predictTitle")) {
            atribuir(&mut m, "_predictTitle", nova.get("_predictTitle"));
        }
        if js::truthy(nova.get("_predictSelection")) {
            atribuir(&mut m, "_predictSelection", nova.get("_predictSelection"));
        }
        let generica = |v: Option<&Value>| matches!(v.and_then(Value::as_str), Some("Outros" | "Outros Esportes"));
        if js::truthy(nova.get("_category"))
            && (!generica(nova.get("_category")) || !js::truthy(antiga.get("_category")) || generica(antiga.get("_category")))
        {
            atribuir(&mut m, "_category", nova.get("_category"));
        }
    }
    // Dados financeiros: a API atual é a fonte de verdade.
    for k in ["realizedPnl", "initialValue", "totalBought", "size", "avgPrice", "cashPnl"] {
        if nova.get(k).is_some_and(|v| !v.is_null()) {
            atribuir(&mut m, k, nova.get(k));
        }
    }
    let v2 = nova.get("_polyDataVersion").and_then(Value::as_f64) == Some(2.0) && nova.get("_polyDataVersion").is_some_and(Value::is_number);
    if v2 && nova.get("status").and_then(Value::as_str) == Some("CLOSED") && e_numero_finito(nova.get("realizedPnl")) {
        if js::truthy(nova.get("_pnlFluxo")) {
            m.insert("_pnlFluxo".into(), json!(true));
            atribuir(&mut m, "_originalApiFinancials", nova.get("_originalApiFinancials"));
        } else {
            // CLOSED v2 sem fluxo comprovado usa o PnL e a base histórica da API.
            if js::truthy(antiga.get("_pnlFluxo")) && nova.get("initialValue").is_none_or(Value::is_null) {
                m.shift_remove("initialValue");
            }
            m.shift_remove("_pnlFluxo");
            m.shift_remove("_originalApiFinancials");
        }
    }
    // Título: só uma revisão versionada do formato Kalshi substitui um título bom.
    let versao = |v: &Value| {
        let n = js::number(v.get("_kalshiTitleFormatVersion"));
        if n.is_nan() { 0.0 } else { n }
    };
    let (versao_antiga, versao_nova) = (versao(antiga), versao(nova));
    let plataforma = if js::truthy(nova.get("_platform")) { nova.get("_platform") } else { antiga.get("_platform") };
    let atualiza_formato =
        texto_truthy(plataforma).to_lowercase() == "kalshi" && js::truthy(nova.get("title")) && versao_nova > versao_antiga;
    if atualiza_formato {
        atribuir(&mut m, "title", nova.get("title"));
        m.insert("_kalshiTitleFormatVersion".into(), js::numero_json(versao_nova));
    } else if js::truthy(antiga.get("title")) && !titulo_fraco(antiga.get("title")) {
        atribuir(&mut m, "title", antiga.get("title"));
    } else if js::truthy(nova.get("title")) {
        atribuir(&mut m, "title", nova.get("title"));
    }
    // A data do jogo pode ser corrigida por outro mercado do mesmo confronto.
    if js::truthy(nova.get("_startDate")) {
        atribuir(&mut m, "_startDate", nova.get("_startDate"));
    } else if js::truthy(antiga.get("_startDate")) {
        atribuir(&mut m, "_startDate", antiga.get("_startDate"));
    }
    if js::number(nova.get("timestamp")) > 0.0 {
        atribuir(&mut m, "timestamp", nova.get("timestamp"));
    } else if js::number(antiga.get("timestamp")) > 0.0 {
        atribuir(&mut m, "timestamp", antiga.get("timestamp"));
    }
    for k in ["_eventTicker", "_matchupKey"] {
        if js::truthy(antiga.get(k)) {
            atribuir(&mut m, k, antiga.get(k));
        }
    }
    if js::truthy(antiga.get("_settledTime")) && !js::truthy(nova.get("_settledTime")) {
        atribuir(&mut m, "_settledTime", antiga.get("_settledTime"));
    }
    if audit::timestamp_ms(antiga.get("_resolvedTime")).is_some_and(|v| v != 0.0) {
        atribuir(&mut m, "_resolvedTime", antiga.get("_resolvedTime"));
    } else if audit::timestamp_ms(nova.get("_resolvedTime")).is_some_and(|v| v != 0.0) {
        atribuir(&mut m, "_resolvedTime", nova.get("_resolvedTime"));
    }
    // Uma modalidade recém-enriquecida vale mais que a categoria raiz "Sports".
    if js::truthy(nova.get("_sportMetaResolved")) {
        let esporte = if js::truthy(nova.get("sport")) { g(nova, "sport") } else { g(antiga, "sport") };
        atribuir(&mut m, "sport", esporte.as_ref());
        m.insert("_sportMetaResolved".into(), json!(true));
    }
    if js::truthy(nova.get("_sportMetaChecked")) {
        m.insert("_sportMetaChecked".into(), json!(true));
    }
    let exato_novo = titles::exact_market(nova, nova.get("_sportMeta"))?.is_some();
    if exato_novo && titles::exact_market(antiga, antiga.get("_sportMeta"))?.is_none() {
        atribuir(&mut m, "_sportMeta", nova.get("_sportMeta"));
    }
    Ok(audit::protect_closed_date(Some(antiga), Value::Object(m)))
}

/// `String.prototype.trim()`: o conjunto de espaços do JS, que inclui o BOM e
/// não inclui o U+0085 (o `str::trim` do Rust faz o contrário nos dois).
fn aparar_js(s: &str) -> &str {
    s.trim_matches(|c: char| {
        matches!(
            c,
            '\u{9}'..='\u{d}' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}'
                | '\u{3000}' | '\u{feff}'
        )
    })
}

/// `.replace(/^"|"$/g, '')`: uma aspa no início e uma no fim.
fn sem_aspas(s: &str) -> &str {
    let s = s.strip_prefix('"').unwrap_or(s);
    s.strip_suffix('"').unwrap_or(s)
}

/// Índice do `selectedMarket` de `normalizarNomeMercadoPolymarket()`, que
/// procura o mercado com `String(...).trim()` em vez do `clean()` do
/// `exactMarket()`.
fn mercado_selecionado_js(aposta: &Value, mercados: &[Value]) -> Option<usize> {
    // `String(a || b || "").trim()`.
    let campo = |v: Option<&Value>| aparar_js(&texto_truthy(v)).to_string();
    let asset = campo(ou3(aposta, ["asset", "assetId", "tokenId"]));
    let condicao = campo(titles::ou(aposta.get("conditionId"), aposta.get("condition_id"))).to_lowercase();
    let slug = campo(aposta.get("slug")).to_lowercase();
    let unico = |achados: Vec<usize>| if achados.len() == 1 { Some(achados[0]) } else { None };
    let mut escolhido = None;
    if !asset.is_empty() {
        let alvo = sem_aspas(&asset);
        // `(Array.isArray(m?.tokenIds) ? m.tokenIds : []).some(t => String(t).trim()...)`.
        let tem_token = |m: &Value| {
            m.get("tokenIds")
                .and_then(Value::as_array)
                .is_some_and(|ids| ids.iter().any(|t| sem_aspas(aparar_js(&js::string(Some(t)))) == alvo))
        };
        escolhido = unico((0..mercados.len()).filter(|&i| tem_token(&mercados[i])).collect());
    }
    if escolhido.is_none() && !condicao.is_empty() {
        let condicao_de = |m: &Value| campo(titles::ou(m.get("conditionId"), m.get("condition_id"))).to_lowercase();
        escolhido = unico((0..mercados.len()).filter(|&i| condicao_de(&mercados[i]) == condicao).collect());
    }
    if escolhido.is_none() && !slug.is_empty() {
        escolhido = unico((0..mercados.len()).filter(|&i| campo(mercados[i].get("slug")).to_lowercase() == slug).collect());
    }
    escolhido
}

/// `a || b || c` sobre campos de um objeto.
fn ou3<'a>(v: &'a Value, chaves: [&str; 3]) -> Option<&'a Value> {
    chaves.iter().map(|k| v.get(*k)).find(|c| js::truthy(*c)).flatten()
}

/// Reduz o `_sportMeta` de uma aposta ao mercado exato dela.
///
/// O Gamma devolve o evento inteiro: centenas de mercados irmãos, cada um com
/// a descrição completa. Depois de achado o mercado exato, ninguém lê os
/// irmãos de novo. Eles só entram em `eventTexts`, que passa a ser gravado já
/// calculado (os textos do evento sem os textos dos mercados, como
/// `sinaisDaAposta()` e `history::compactar` fazem). `texts` e o resto do
/// metadado ficam como estão.
///
/// Só compacta quando os dois jeitos de achar o mercado (o `exactMarket()` e o
/// `selectedMarket` dos títulos) apontam para o mesmo mercado e a forma que a
/// tabela recebe não muda. Em qualquer outro caso a aposta fica intacta.
/// Devolve `true` quando mudou.
pub fn compactar_meta_esporte(aposta: &mut Value) -> bool {
    let Some(meta) = aposta.get("_sportMeta").filter(|m| m.is_object()) else { return false };
    let Some(mercados) = meta.get("markets").and_then(Value::as_array) else { return false };
    if mercados.len() <= 1 || !mercados.iter().all(Value::is_object) {
        return false;
    }
    let Ok(Some(exato)) = titles::exact_market(aposta, Some(meta)) else { return false };
    let Some(indice) = mercados.iter().position(|m| std::ptr::eq(m, exato)) else { return false };
    if mercado_selecionado_js(aposta, mercados) != Some(indice) {
        return false;
    }
    let textos_evento = match meta.get("eventTexts") {
        Some(Value::Array(_)) => None,
        _ => {
            let textos_mercado: HashSet<String> = mercados
                .iter()
                .flat_map(|m| ["slug", "question", "groupItemTitle", "sportsMarketType"].map(|k| m.get(k)))
                .filter(|v| js::truthy(*v))
                .map(|v| js::string(v).to_lowercase())
                .collect();
            let lista = match meta.get("texts") {
                Some(Value::Array(a)) => {
                    a.iter().filter(|v| !textos_mercado.contains(&js::string(Some(v)).to_lowercase())).cloned().collect()
                }
                _ => Vec::new(),
            };
            Some(Value::Array(lista))
        }
    };
    let mut novo = meta.clone();
    if let Some(o) = novo.as_object_mut() {
        o.insert("markets".into(), Value::Array(vec![exato.clone()]));
        if let Some(t) = textos_evento {
            o.insert("eventTexts".into(), t);
        }
    }
    let tabela_antes = history::meta_da_tabela(aposta);
    let antigo = std::mem::replace(aposta.get_mut("_sportMeta").expect("_sportMeta conferido acima"), novo);
    if tabela_antes.is_none() || history::meta_da_tabela(aposta) != tabela_antes {
        aposta["_sportMeta"] = antigo;
        return false;
    }
    true
}

/// Resultado da compactação única dos metadados já gravados.
#[derive(Debug, Default)]
pub struct Compactacao {
    pub apostas: usize,
    pub compactadas: usize,
    pub bytes_antes: usize,
    pub bytes_depois: usize,
}

const MANUTENCAO_COMPACTACAO: &str = "compact_polymarket_sport_meta_v1";

/// Compacta, uma única vez, o `_sportMeta` das apostas gravadas antes de a
/// escrita compactar. Tudo numa transação: se o app fechar no meio, a próxima
/// abertura repete. `updated_at` não muda, porque a aposta continua a mesma.
pub fn compactar_ledger_uma_vez(c: &mut Connection) -> Result<Option<Compactacao>> {
    let feito = c
        .query_row("SELECT 1 FROM database_maintenance WHERE maintenance_key=?", [MANUTENCAO_COMPACTACAO], |_| Ok(()))
        .optional()?
        .is_some();
    if feito {
        return Ok(None);
    }
    let ids: Vec<i64> = {
        let mut s = c.prepare("SELECT rowid FROM closed_bets_ledger WHERE platform='polymarket' ORDER BY rowid")?;
        s.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?
    };
    let mut r = Compactacao::default();
    let tx = c.transaction()?;
    {
        let mut ler = tx.prepare("SELECT data_json FROM closed_bets_ledger WHERE rowid=?")?;
        let mut regravar = tx.prepare("UPDATE closed_bets_ledger SET data_json=? WHERE rowid=?")?;
        for id in ids {
            let texto: String = ler.query_row([id], |l| l.get(0))?;
            r.apostas += 1;
            r.bytes_antes += texto.len();
            let novo = serde_json::from_str::<Value>(&texto)
                .ok()
                .and_then(|mut v| compactar_meta_esporte(&mut v).then(|| js::para_texto(&v)));
            match novo {
                Some(n) => {
                    r.bytes_depois += n.len();
                    r.compactadas += 1;
                    regravar.execute(params![n, id])?;
                }
                None => r.bytes_depois += texto.len(),
            }
        }
    }
    tx.execute(
        "INSERT OR REPLACE INTO database_maintenance(maintenance_key,completed_at) VALUES(?,?)",
        params![MANUTENCAO_COMPACTACAO, agora()],
    )?;
    tx.commit()?;
    Ok(Some(r))
}

/// Campos de transporte que nunca entram no JSON canônico do ledger.
const CAMPOS_TRANSITORIOS: [&str; 4] = ["_ledgerSourceSignature", "_kalshiTitleMeta", "_titleDisplay", "_titleDisplayVersion"];

pub struct Gravacao {
    pub recebidas: usize,
    pub gravadas: usize,
    pub inalteradas: usize,
    /// Índices, em `processadas`, das linhas que foram (re)gravadas.
    pub alteradas: Vec<usize>,
    pub processadas: Vec<Value>,
}

fn mapa_existente(c: &Connection, user_id: &str, linhas: &[Value], incremental: bool) -> Result<HashMap<String, String>> {
    let mut mapa = HashMap::new();
    if !incremental {
        let mut s = c.prepare("SELECT platform,bet_key,data_json FROM closed_bets_ledger WHERE user_id=?")?;
        for r in s.query_map([user_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))? {
            let (p, k, d) = r?;
            mapa.insert(format!("{p}|{k}"), d);
        }
        return Ok(mapa);
    }
    // No caminho incremental só lê as chaves que chegaram no lote.
    let mut por_casa: Vec<(&str, Vec<String>)> = Vec::new();
    for linha in linhas {
        let casa = casa_historico(linha.get("_platform"));
        let chave = chave_com_casa(linha, casa);
        if !chave_valida(&chave) {
            continue;
        }
        match por_casa.iter_mut().find(|(c, _)| *c == casa) {
            Some((_, lista)) => {
                if !lista.contains(&chave) {
                    lista.push(chave)
                }
            }
            None => por_casa.push((casa, vec![chave])),
        }
    }
    for (casa, chaves) in por_casa {
        for lote in chaves.chunks(400) {
            let marcas = vec!["?"; lote.len()].join(",");
            let sql = format!("SELECT platform,bet_key,data_json FROM closed_bets_ledger WHERE user_id=? AND platform=? AND bet_key IN ({marcas})");
            let mut s = c.prepare(&sql)?;
            let mut p: Vec<&dyn ToSql> = vec![&user_id, &casa];
            p.extend(lote.iter().map(|k| k as &dyn ToSql));
            for r in s.query_map(p.as_slice(), |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))? {
                let (pl, k, d) = r?;
                mapa.insert(format!("{pl}|{k}"), d);
            }
        }
    }
    Ok(mapa)
}

/// `closedLedgerKey({...row, _platform: casa})`, sem copiar a linha.
fn chave_com_casa(linha: &Value, casa: &str) -> String {
    chave_ledger_casa(linha, casa)
}

/// `saveClosedLedger()`.
pub fn gravar(c: &mut Connection, user_id: &str, linhas: Vec<Value>, incremental: bool) -> Result<Gravacao> {
    let normalizadas: Vec<Value> = linhas
        .into_iter()
        .map(|l| {
            let mut o = match l {
                Value::Object(o) => o,
                _ => Map::new(),
            };
            for k in CAMPOS_TRANSITORIOS {
                o.shift_remove(k);
            }
            Value::Object(o)
        })
        .collect();
    let mut existente = mapa_existente(c, user_id, &normalizadas, incremental)?;
    let mut g = Gravacao { recebidas: normalizadas.len(), gravadas: 0, inalteradas: 0, alteradas: vec![], processadas: vec![] };
    let tx = c.savepoint()?;
    {
        let mut upsert = tx.prepare(
            "INSERT INTO closed_bets_ledger(user_id,platform,bet_key,data_json,first_seen_at,updated_at) VALUES(?,?,?,?,?,?)
    ON CONFLICT(user_id,platform,bet_key) DO UPDATE SET data_json=excluded.data_json,updated_at=excluded.updated_at",
        )?;
        for linha in normalizadas {
            let casa = casa_historico(linha.get("_platform"));
            let chave = chave_com_casa(&linha, casa);
            if !chave_valida(&chave) {
                continue;
            }
            let chave_mapa = format!("{casa}|{chave}");
            let json_existente = existente.remove(&chave_mapa);
            let mesclada = json_existente
                .as_deref()
                .and_then(|texto| serde_json::from_str::<Value>(texto).ok())
                .and_then(|antiga| merge(&antiga, &linha).ok());
            let mut dado = match mesclada {
                Some(m) => m,
                None => audit::protect_closed_date(None, linha),
            };
            compactar_meta_esporte(&mut dado);
            let texto = js::para_texto(&dado);
            // Resultado idêntico ao salvo: não regrava.
            if json_existente.as_deref() == Some(texto.as_str()) {
                g.inalteradas += 1;
                existente.insert(chave_mapa, texto);
                g.processadas.push(dado);
                continue;
            }
            let t = agora();
            upsert.execute(params![user_id, casa, chave, texto, t, t])?;
            existente.insert(chave_mapa, texto);
            g.alteradas.push(g.processadas.len());
            g.processadas.push(dado);
        }
    }
    tx.commit()?;
    g.gravadas = g.alteradas.len();
    Ok(g)
}

fn numero_ou(v: Option<&Value>, padrao: f64) -> f64 {
    let n = js::number(v);
    if n.is_finite() { n } else { padrao }
}

/// `String(v||'').slice(0, n)` contando UTF-16 como o JS.
fn texto_cortado(v: Option<&Value>, n: usize) -> String {
    js::fatia_utf16(&texto_truthy(v), n).to_string()
}

fn ymd_valido(v: Option<&Value>) -> String {
    let s = texto_truthy(v);
    if js::e_data_ymd(&s) { s } else { String::new() }
}

/// `saveClosedHistoryIndex()`: só indexa apostas que já estão no ledger.
pub fn salvar_indice(c: &mut Connection, user_id: &str, entradas: Option<&Value>) -> Result<(usize, usize)> {
    let lista = entradas.and_then(Value::as_array).cloned().unwrap_or_default();
    struct Entrada {
        casa: &'static str,
        chave: String,
        categoria: String,
        inicio: String,
        tabela: String,
        tabela_ts: f64,
        pnl: f64,
        stake: f64,
        odd: f64,
        venceu: i64,
        assinatura: String,
        busca: String,
        titulo: String,
        versao: String,
    }
    let linhas: Vec<Entrada> = lista
        .iter()
        .filter_map(|raw| {
            let chave = texto_truthy(titles::ou(raw.get("betKey"), raw.get("bet_key"))).trim().to_string();
            if chave.is_empty() {
                return None;
            }
            let tabela_ts = js::number(raw.get("tableTs"));
            Some(Entrada {
                casa: casa_historico(titles::ou(raw.get("platform"), raw.get("_platform"))),
                chave,
                categoria: {
                    let c = raw.get("category");
                    js::fatia_utf16(&if js::truthy(c) { js::string(c) } else { "Outros".into() }, 80).to_string()
                },
                inicio: ymd_valido(raw.get("startYmd")),
                tabela: ymd_valido(raw.get("tableYmd")),
                tabela_ts: if tabela_ts.is_finite() && tabela_ts > 0.0 { crate::odds::js_round(tabela_ts) } else { 0.0 },
                pnl: numero_ou(raw.get("pnlValue"), 0.0),
                stake: numero_ou(raw.get("stakeValue"), 0.0).max(0.0),
                odd: numero_ou(raw.get("oddValue"), 0.0).max(0.0),
                venceu: i64::from(numero_ou(raw.get("won"), 0.0) > 0.0),
                assinatura: texto_cortado(raw.get("sourceSignature"), 4096),
                busca: js::fatia_utf16(&texto_truthy(raw.get("titleSearch")).to_lowercase(), 1000).to_string(),
                titulo: texto_cortado(raw.get("titleDisplay"), 1000),
                versao: texto_cortado(raw.get("titleVersion"), 40),
            })
        })
        .collect();
    if linhas.is_empty() {
        return Ok((0, 0));
    }
    let recebidas = linhas.len();
    let mut gravadas = 0;
    let tx = c.savepoint()?;
    {
        let mut existente = tx.prepare("SELECT data_json FROM closed_bets_ledger WHERE user_id=? AND platform=? AND bet_key=?")?;
        let mut upsert = tx.prepare(
            "INSERT INTO closed_bets_index(user_id,platform,bet_key,category,start_ymd,table_ymd,table_ts,pnl_ts,pnl_ymd,pnl_value,stake_value,odd_value,won,source_signature,metrics_ready,title_search,title_display,updated_at)
    VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
    ON CONFLICT(user_id,platform,bet_key) DO UPDATE SET
      category=excluded.category,start_ymd=excluded.start_ymd,table_ymd=excluded.table_ymd,table_ts=excluded.table_ts,
      pnl_ts=excluded.pnl_ts,pnl_ymd=excluded.pnl_ymd,pnl_value=excluded.pnl_value,stake_value=excluded.stake_value,
      odd_value=excluded.odd_value,won=excluded.won,source_signature=excluded.source_signature,metrics_ready=excluded.metrics_ready,
      title_search=excluded.title_search,title_display=excluded.title_display,updated_at=excluded.updated_at",
        )?;
        let mut estado_titulo =
            tx.prepare("UPDATE closed_bets_index SET title_display=?,title_search=?,title_version=? WHERE user_id=? AND platform=? AND bet_key=?")?;
        let mut titulo_anterior =
            tx.prepare("SELECT title_display,title_search,title_version FROM closed_bets_index WHERE user_id=? AND platform=? AND bet_key=?")?;
        for e in linhas {
            // Nunca cria índice órfão: só apostas confirmadas no ledger.
            let Some(canonico) = existente.query_row(params![user_id, e.casa, e.chave], |r| r.get::<_, String>(0)).optional()? else {
                continue;
            };
            let mut dado: Value = serde_json::from_str(&canonico)?;
            if let Some(o) = dado.as_object_mut() {
                o.insert("_platform".into(), Value::String(e.casa.into()));
            }
            let pnl_ts = audit::pnl_timestamp(&dado, None);
            let pnl_ymd = if pnl_ts != 0.0 { ymd_sao_paulo(pnl_ts) } else { String::new() };
            let anterior: Option<(String, String, String)> = if e.casa == "kalshi" {
                titulo_anterior.query_row(params![user_id, e.casa, e.chave], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional()?
            } else {
                None
            };
            upsert.execute(params![
                user_id, e.casa, e.chave, e.categoria, e.inicio, e.tabela, e.tabela_ts, pnl_ts, pnl_ymd, e.pnl, e.stake, e.odd, e.venceu,
                e.assinatura, 1, e.busca, e.titulo, agora()
            ])?;
            if e.casa == "kalshi" {
                match anterior.filter(|a| e.versao.is_empty() && !a.2.is_empty()) {
                    Some((titulo, busca, versao)) => estado_titulo.execute(params![titulo, busca, versao, user_id, e.casa, e.chave])?,
                    None => estado_titulo.execute(params![e.titulo, e.busca, e.versao, user_id, e.casa, e.chave])?,
                };
            }
            gravadas += 1;
        }
    }
    tx.commit()?;
    Ok((recebidas, gravadas))
}

/// `atualizarTitulosIndiceHistorico()`.
pub fn atualizar_titulos(c: &mut Connection, user_id: &str, entradas: Option<&Value>) -> Result<(usize, usize)> {
    let lista = entradas.and_then(Value::as_array).cloned().unwrap_or_default();
    let linhas: Vec<(&'static str, String, String, String, String)> = lista
        .iter()
        .map(|raw| {
            (
                casa_historico(titles::ou(raw.get("platform"), raw.get("_platform"))),
                texto_truthy(titles::ou(raw.get("betKey"), raw.get("bet_key"))).trim().to_string(),
                js::fatia_utf16(&texto_truthy(raw.get("titleSearch")).to_lowercase(), 1000).to_string(),
                texto_cortado(raw.get("titleDisplay"), 1000),
                texto_cortado(raw.get("titleVersion"), 40),
            )
        })
        .filter(|l| !l.1.is_empty())
        .collect();
    if linhas.is_empty() {
        return Ok((0, 0));
    }
    let recebidas = linhas.len();
    let mut atualizadas = 0;
    let tx = c.transaction()?;
    {
        let mut update = tx.prepare(
            "UPDATE closed_bets_index SET title_search=?,title_display=?,updated_at=?
    WHERE user_id=? AND platform=? AND bet_key=? AND (title_search<>? OR title_display<>?)",
        )?;
        let mut confirmar = tx.prepare("UPDATE closed_bets_index SET title_version=? WHERE user_id=? AND platform=? AND bet_key=?")?;
        for (casa, chave, busca, titulo, versao) in linhas {
            if casa == "kalshi" && versao.is_empty() {
                continue;
            }
            atualizadas += update.execute(params![busca, titulo, agora(), user_id, casa, chave, busca, titulo])?;
            if casa == "kalshi" {
                confirmar.execute(params![versao, user_id, casa, chave])?;
            }
        }
    }
    tx.commit()?;
    Ok((recebidas, atualizadas))
}

/// `readClosedLedgerSignatures()`: assinaturas compactas das chaves pedidas.
pub fn assinaturas(c: &Connection, user_id: &str, entradas: Option<&Value>) -> Result<Vec<Value>> {
    let mut grupos: Vec<(&'static str, Vec<String>)> = Vec::new();
    for raw in entradas.and_then(Value::as_array).map_or(&[][..], |a| a.as_slice()) {
        let casa = casa_historico(raw.get("platform"));
        let chave = texto_truthy(titles::ou(raw.get("betKey"), raw.get("bet_key"))).trim().to_string();
        if chave.is_empty() {
            continue;
        }
        match grupos.iter_mut().find(|(c, _)| *c == casa) {
            Some((_, l)) => {
                if !l.contains(&chave) {
                    l.push(chave)
                }
            }
            None => grupos.push((casa, vec![chave])),
        }
    }
    let mut saida = Vec::new();
    for (casa, chaves) in grupos {
        for lote in chaves.chunks(400) {
            let marcas = vec!["?"; lote.len()].join(",");
            let sql = format!(
                "SELECT platform,bet_key,source_signature,metrics_ready,title_version FROM closed_bets_index WHERE user_id=? AND platform=? AND bet_key IN ({marcas})"
            );
            let mut s = c.prepare(&sql)?;
            let mut p: Vec<&dyn ToSql> = vec![&user_id, &casa];
            p.extend(lote.iter().map(|k| k as &dyn ToSql));
            let rows = s.query_map(p.as_slice(), |r| {
                Ok(json!({
                    "platform": r.get::<_, String>(0)?,
                    "betKey": r.get::<_, String>(1)?,
                    "sourceSignature": r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    "metricsReady": r.get::<_, i64>(3)? != 0,
                    "titleVersion": r.get::<_, Option<String>>(4)?.unwrap_or_default()
                }))
            })?;
            for r in rows {
                saida.push(r?);
            }
        }
    }
    Ok(saida)
}

/// `POST /api/history/closed/metadata`: só metadados da Polymarket, nunca valores.
pub fn metadados(c: &Connection, user_id: &str, chaves: Option<&Value>) -> Result<Vec<Value>> {
    let mut unicas: Vec<String> = Vec::new();
    for v in chaves.and_then(Value::as_array).map_or(&[][..], |a| a.as_slice()) {
        let s = js::string(Some(v));
        if !unicas.contains(&s) {
            unicas.push(s);
        }
    }
    unicas.truncate(400);
    if unicas.is_empty() {
        return Ok(vec![]);
    }
    let marcas = vec!["?"; unicas.len()].join(",");
    let sql = format!("SELECT bet_key,data_json FROM closed_bets_ledger WHERE user_id=? AND platform='polymarket' AND bet_key IN ({marcas})");
    let mut s = c.prepare(&sql)?;
    let mut p: Vec<&dyn ToSql> = vec![&user_id];
    p.extend(unicas.iter().map(|k| k as &dyn ToSql));
    let mut saida = Vec::new();
    for r in s.query_map(p.as_slice(), |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (chave, data) = r?;
        let d: Linha = serde_json::from_str(&data)?;
        let mut e = Map::new();
        e.insert("betKey".into(), Value::String(chave));
        for k in ["eventSlug", "slug", "_sportMeta", "_startDate", "_dateSource", "_dateSuspect", "_resolvedTime"] {
            if let Some(v) = d.get(k) {
                e.insert(k.into(), serde_json::from_str(v.get())?);
            }
        }
        saida.push(Value::Object(e));
    }
    Ok(saida)
}

fn chave_kalshi(l: &Value) -> String {
    texto_truthy(["slug", "ticker", "market_ticker"].iter().map(|k| l.get(*k)).find(|v| js::truthy(*v)).flatten())
}

fn para_linha(v: &Value) -> Result<Linha> {
    Ok(serde_json::from_str(&js::para_texto(v))?)
}

fn de_linha(l: &Linha) -> Result<Value> {
    Ok(serde_json::from_str(&js::para_texto(l))?)
}

/// `prepararTitulosKalshiDelta()`: devolve o título já indexado ou os
/// metadados para o navegador recalcular o título Kalshi.
pub fn preparar_titulos_kalshi(c: &Connection, user_id: &str, linhas: Vec<Value>, versao: Option<&Value>) -> Result<Vec<Value>> {
    let revisao = texto_cortado(versao, 40);
    let mut anteriores: HashMap<String, (String, String)> = HashMap::new();
    let chaves: Vec<String> = {
        let mut v: Vec<String> = Vec::new();
        let mut vistas = HashSet::new();
        for l in linhas.iter().filter(|l| l.get("_platform").and_then(Value::as_str) == Some("kalshi")) {
            let k = chave_kalshi(l);
            if !k.is_empty() && vistas.insert(k.clone()) {
                v.push(k);
            }
        }
        v
    };
    for lote in chaves.chunks(400) {
        let marcas = vec!["?"; lote.len()].join(",");
        let sql = format!("SELECT bet_key,title_display,title_version FROM closed_bets_index WHERE user_id=? AND platform='kalshi' AND bet_key IN ({marcas})");
        let mut s = c.prepare(&sql)?;
        let mut p: Vec<&dyn ToSql> = vec![&user_id];
        p.extend(lote.iter().map(|k| k as &dyn ToSql));
        for r in s.query_map(p.as_slice(), |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(2)?)))? {
            let (k, t, v) = r?;
            anteriores.insert(k, (t.unwrap_or_default(), v.unwrap_or_default()));
        }
    }
    let titulo_de = |l: &Value, anterior: Option<&(String, String)>| -> String {
        match anterior.filter(|a| !a.0.is_empty()) {
            Some(a) => a.0.clone(),
            None => texto_truthy(l.get("title")),
        }
    };
    let pendentes: Vec<&Value> = linhas
        .iter()
        .filter(|l| l.get("_platform").and_then(Value::as_str) == Some("kalshi"))
        .filter(|l| {
            let a = anteriores.get(&chave_kalshi(l));
            !(!revisao.is_empty() && a.is_some_and(|a| a.1 == revisao)) || titles::kalshi_title_missing_matchup(l, Some(&titulo_de(l, a)))
        })
        .collect();
    let mut enriquecidas: HashMap<String, Value> = HashMap::new();
    let convertidas: Result<Vec<Linha>> = pendentes.iter().map(|l| para_linha(l)).collect();
    if let Ok(com_meta) = convertidas.and_then(|l| history::anexar_metadados_kalshi(c, l)) {
        for l in &com_meta {
            if let Ok(v) = de_linha(l) {
                enriquecidas.insert(chave_kalshi(&v), v);
            }
        }
    }
    Ok(linhas
        .into_iter()
        .map(|linha| {
            if linha.get("_platform").and_then(Value::as_str) != Some("kalshi") {
                return linha;
            }
            let chave = chave_kalshi(&linha);
            let anterior = anteriores.get(&chave);
            let com = |titulo: &str, versao: &str| -> Value {
                let mut o = linha.as_object().cloned().unwrap_or_default();
                o.insert("_titleDisplay".into(), Value::String(titulo.into()));
                o.insert("_titleDisplayVersion".into(), Value::String(versao.into()));
                Value::Object(o)
            };
            if let Some(a) = anterior {
                if !revisao.is_empty() && a.1 == revisao && !a.0.is_empty() && !titles::kalshi_title_missing_matchup(&linha, Some(&a.0)) {
                    return com(&a.0, &revisao);
                }
            }
            if let Some(fresca) = enriquecidas.get(&chave).filter(|f| js::truthy(f.get("_kalshiTitleMeta"))) {
                return fresca.clone();
            }
            if let Some(a) = anterior.filter(|a| !a.0.is_empty()) {
                return com(&a.0, &a.1);
            }
            linha
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Aposta da Polymarket com o evento inteiro no `_sportMeta`.
    fn aposta_com_evento() -> Value {
        json!({
            "_platform": "polymarket", "conditionId": "0xC2", "asset": "222", "outcome": "Over", "slug": "jogo-total-2pt5",
            "title": "Time A vs. Time B: O/U 2.5",
            "_sportMeta": {
                "texts": ["Time A vs. Time B", "Soccer", "jogo-ml", "Will Time A win?", "jogo-total-2pt5", "Time A vs. Time B: O/U 2.5", "soccer_totals"],
                "sportsMarketTypes": ["moneyline", "soccer_totals"],
                "markets": [
                    { "conditionId": "0xC1", "slug": "jogo-ml", "question": "Will Time A win?", "description": "longa".repeat(50), "tokenIds": ["111", "112"] },
                    { "conditionId": "0xC2", "slug": "jogo-total-2pt5", "question": "Time A vs. Time B: O/U 2.5", "description": "regras",
                      "sportsMarketType": "soccer_totals", "line": 2.5, "tokenIds": ["222", "223"] }
                ],
                "eventId": "77", "seriesSlug": null
            }
        })
    }

    #[test]
    fn compacta_o_evento_no_mercado_exato() {
        let mut a = aposta_com_evento();
        let tabela_antes = history::meta_da_tabela(&a);
        assert!(compactar_meta_esporte(&mut a));
        let meta = &a["_sportMeta"];
        assert_eq!(meta["markets"].as_array().unwrap().len(), 1);
        assert_eq!(meta["markets"][0]["conditionId"], "0xC2");
        assert_eq!(meta["markets"][0]["description"], "regras", "o mercado exato fica inteiro");
        assert_eq!(meta["texts"].as_array().unwrap().len(), 7, "texts não muda");
        assert_eq!(meta["eventTexts"], json!(["Time A vs. Time B", "Soccer"]), "textos do evento sem os dos mercados");
        assert_eq!(meta["sportsMarketTypes"], json!(["moneyline", "soccer_totals"]));
        assert_eq!(meta["eventId"], "77");
        assert!(meta.as_object().unwrap().contains_key("seriesSlug"));
        assert_eq!(history::meta_da_tabela(&a), tabela_antes, "a tabela recebe o mesmo metadado");
        let uma_vez = js::para_texto(&a);
        assert!(!compactar_meta_esporte(&mut a), "compactar de novo não muda nada");
        assert_eq!(js::para_texto(&a), uma_vez);
    }

    #[test]
    fn eventtexts_existente_e_mantido() {
        let mut a = aposta_com_evento();
        a["_sportMeta"]["eventTexts"] = json!(["Futebol"]);
        assert!(compactar_meta_esporte(&mut a));
        assert_eq!(a["_sportMeta"]["eventTexts"], json!(["Futebol"]));
    }

    #[test]
    fn sem_mercado_unico_a_aposta_fica_intacta() {
        type Mudanca = Box<dyn Fn(&mut Value)>;
        let casos: Vec<Mudanca> = vec![
            // Nenhum mercado com o asset, a condição ou o slug da aposta.
            Box::new(|a| {
                a["asset"] = json!("999");
                a["conditionId"] = json!("0xZZ");
                a["slug"] = json!("outro");
            }),
            // Dois mercados com o mesmo asset e a mesma condição.
            Box::new(|a| {
                a["_sportMeta"]["markets"][0]["tokenIds"] = json!(["222"]);
                a["_sportMeta"]["markets"][0]["conditionId"] = json!("0xC2");
                a["_sportMeta"]["markets"][0]["slug"] = json!("jogo-total-2pt5");
            }),
            // Mercado nulo: o JS lançaria exceção.
            Box::new(|a| a["_sportMeta"]["markets"][0] = Value::Null),
            // Um só mercado: não há o que tirar.
            Box::new(|a| {
                a["_sportMeta"]["markets"].as_array_mut().unwrap().remove(0);
            }),
            // `clean()` acha o mercado 1 pelo asset; o `String().trim()` dos
            // títulos não acha e cai na condição, que é a do mercado 0.
            Box::new(|a| {
                a["asset"] = json!("22 2");
                a["conditionId"] = json!("0xC1");
                a["_sportMeta"]["markets"][1]["tokenIds"] = json!(["22  2"]);
            }),
            Box::new(|a| a["_sportMeta"] = json!("texto")),
        ];
        for (i, mudar) in casos.iter().enumerate() {
            let mut a = aposta_com_evento();
            mudar(&mut a);
            let antes = js::para_texto(&a);
            assert!(!compactar_meta_esporte(&mut a), "caso {i}");
            assert_eq!(js::para_texto(&a), antes, "caso {i}");
        }
    }

    #[test]
    fn gravar_compacta_e_o_merge_mantem_compactado() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = crate::db::abrir(&dir.path().join("t.db")).unwrap();
        let u = crate::db::usuario_local(&mut c).unwrap();
        let g = gravar(&mut c, &u.id, vec![aposta_com_evento()], true).unwrap();
        assert_eq!(g.gravadas, 1);
        assert_eq!(g.processadas[0]["_sportMeta"]["markets"].as_array().unwrap().len(), 1, "a resposta já vem compactada");
        let salvo: String = c.query_row("SELECT data_json FROM closed_bets_ledger", [], |r| r.get(0)).unwrap();
        assert!(!salvo.contains("longa"));
        // A fonte manda o evento inteiro de novo: nada muda no ledger.
        let g = gravar(&mut c, &u.id, vec![aposta_com_evento()], true).unwrap();
        assert_eq!((g.gravadas, g.inalteradas), (0, 1));
        let depois: String = c.query_row("SELECT data_json FROM closed_bets_ledger", [], |r| r.get(0)).unwrap();
        assert_eq!(depois, salvo);
    }

    #[test]
    fn compacta_o_ledger_existente_uma_vez() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = crate::db::abrir(&dir.path().join("t.db")).unwrap();
        let u = crate::db::usuario_local(&mut c).unwrap();
        let cheio = js::para_texto(&aposta_com_evento());
        let t = agora();
        c.execute(
            "INSERT INTO closed_bets_ledger(user_id,platform,bet_key,data_json,first_seen_at,updated_at) VALUES(?,?,?,?,?,?)",
            params![u.id, "polymarket", "0xC2|222|Over", cheio, t, t],
        )
        .unwrap();
        let r = compactar_ledger_uma_vez(&mut c).unwrap().expect("primeira vez compacta");
        assert_eq!((r.apostas, r.compactadas), (1, 1));
        assert!(r.bytes_depois < r.bytes_antes);
        let (salvo, atualizado): (String, String) =
            c.query_row("SELECT data_json, updated_at FROM closed_bets_ledger", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(atualizado, t, "a compactação não conta como mudança da aposta");
        let mut esperado = aposta_com_evento();
        compactar_meta_esporte(&mut esperado);
        assert_eq!(salvo, js::para_texto(&esperado));
        assert!(compactar_ledger_uma_vez(&mut c).unwrap().is_none(), "só roda uma vez");
    }

    /// Roda a compactação única numa CÓPIA do banco real, para o oráculo
    /// comparar a cópia original com a compactada.
    /// `BETTY_COMPACTAR_DB=<cópia> cargo test --release -- --ignored compactar_copia_real`.
    #[test]
    #[ignore]
    fn compactar_copia_real() {
        let caminho = std::env::var("BETTY_COMPACTAR_DB").unwrap();
        let mut c = crate::db::abrir(std::path::Path::new(&caminho)).unwrap();
        let inicio = std::time::Instant::now();
        let r = compactar_ledger_uma_vez(&mut c).unwrap();
        let meio = inicio.elapsed();
        let livre = crate::db::recuperar_espaco(&c).unwrap();
        println!("{r:?} compactação={meio:?} vacuum={:?} livre={livre:?}", inicio.elapsed() - meio);
    }

    /// Compara com o JS original pelo TEXTO JSON resultante.
    /// `BETTY_ORACULO_LEDGER=<jsonl> cargo test -- --ignored oraculo_ledger`.
    #[test]
    #[ignore]
    fn oraculo_ledger() {
        let texto = std::fs::read_to_string(std::env::var("BETTY_ORACULO_LEDGER").unwrap()).unwrap();
        let (mut total, mut erros, mut por_fn) = (0usize, Vec::new(), HashMap::<String, usize>::new());
        for linha in texto.lines() {
            let caso: Value = serde_json::from_str(linha).unwrap();
            if caso.get("erro").is_some() {
                continue;
            }
            let f = caso["fn"].as_str().unwrap();
            let a = &caso["args"];
            let rust: Value = match f {
                "closedLedgerKey" => json!(chave_ledger(&a[0])),
                "tituloFracoLedger" => json!(titulo_fraco(Some(&a[0]).filter(|v| !v.is_null()))),
                "pnlTimestamp" => js::numero_json(audit::pnl_timestamp(&a[0], None)),
                "timestampMs" => audit::timestamp_ms(Some(&a[0])).map_or(Value::Null, js::numero_json),
                "protectClosedDate" => json!(js::para_texto(&audit::protect_closed_date(Some(&a[0]).filter(|v| !v.is_null()), a[1].clone()))),
                "ledgerMerge" => match merge(&a[0], &a[1]) {
                    Ok(v) => json!(js::para_texto(&v)),
                    Err(()) => json!("<erro>"),
                },
                outro => panic!("{outro}"),
            };
            total += 1;
            if rust != caso["out"] {
                *por_fn.entry(f.into()).or_default() += 1;
                if erros.len() < 12 {
                    let (r, j) = (rust.to_string(), caso["out"].to_string());
                    let i = r.bytes().zip(j.bytes()).position(|(x, y)| x != y).unwrap_or(0).saturating_sub(60);
                    erros.push(format!("{f}\n   rust …{}\n   js   …{}", &r[i.min(r.len())..(i + 260).min(r.len())], &j[i.min(j.len())..(i + 260).min(j.len())]));
                }
            }
        }
        for e in &erros {
            eprintln!("{e}");
        }
        eprintln!("divergências: {por_fn:?}");
        assert!(por_fn.is_empty(), "{} de {total} divergem", por_fn.values().sum::<usize>());
        eprintln!("{total} casos com texto JSON idêntico ao JS");
    }

    #[test]
    fn chaves_e_titulo_fraco() {
        assert_eq!(chave_ledger(&json!({"_platform": "kalshi", "slug": " KX-1 "})), "KX-1");
        assert_eq!(chave_ledger(&json!({"conditionId": "0xa", "asset": "1", "outcomeIndex": 0})), "0xa|1|0");
        assert_eq!(chave_ledger(&json!({"_platform": "outra", "conditionId": "0xa", "outcome": "Yes"})), "0xa||Yes");
        assert!(titulo_fraco(Some(&json!("Trier vs Leipzig Winner?"))));
        assert!(titulo_fraco(Some(&json!("Will it rain?"))));
        assert!(!titulo_fraco(Some(&json!("Lakers vs Celtics - Lakers ML"))));
    }

    #[test]
    fn merge_preserva_titulo_e_ordem() {
        let antiga = json!({"title": "A vs B - A ML", "_platform": "kalshi", "realizedPnl": 1, "_settledTime": "2026-11-20T10:00:00Z", "slug": "KX"});
        let nova = json!({"title": "A vs B Winner?", "_platform": "kalshi", "realizedPnl": 2, "slug": "KX", "x": 1});
        let m = merge(&antiga, &nova).unwrap();
        assert_eq!(m["title"], "A vs B - A ML");
        assert_eq!(m["realizedPnl"], 2);
        let chaves: Vec<&str> = m.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(chaves, ["title", "_platform", "realizedPnl", "slug", "x", "_settledTime", "_confirmedClosedTs"]);
    }
}
