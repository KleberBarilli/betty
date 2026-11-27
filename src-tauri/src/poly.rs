//! Polymarket, igual ao `server.js` + `lib/polymarket-data-v2.js` +
//! `lib/polymarket-activity-cache.js` + `lib/polymarket-metadata-cache.js`.
//!
//! A carteira vem sempre do banco cifrado, nunca do navegador. Só leitura:
//! Data API v2 (com o contrato antigo de arrays preservado para o frontend),
//! Gamma, saldo pUSD na Polygon e um cache de metadados públicos.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use indexmap::IndexMap;
use rusqlite::OptionalExtension;
use serde_json::{Map, Value, json};
use tauri::http::StatusCode;

use crate::api::{ApiResponse, Estado};
use crate::db::{Db, agora};
use crate::{js, net};

const DATA_V2: &str = "https://data-api.polymarket.com/v2";
pub const GAMMA: &str = "https://gamma-api.polymarket.com";
const POLYGON_RPC: &str = "https://polygon-bor-rpc.publicnode.com";
const PUSD: &str = "0xC011a7E12a19f7B1f670d46F03B03f3342E82DFB";
const TTL_SNAPSHOT_MS: f64 = 60_000.0;
const MAX_SNAPSHOTS_BYTES: usize = 64 * 1024 * 1024;
/// Teto das respostas do Gamma guardadas pelo proxy.
const MAX_PROXY_BYTES: usize = 16 * 1024 * 1024;
const TTL_PROXY: Duration = Duration::from_secs(45);
const TTL_SALDO: Duration = Duration::from_secs(30);
const RECHECAGEM_ATIVIDADE_MS: f64 = 10.0 * 60_000.0;
const MAX_ENTRADA_ATIVIDADE: usize = 16 * 1024 * 1024;
const META_VERSAO: i64 = 1;
const META_TTL_MS: f64 = 7.0 * 86_400_000.0;

struct Snapshot {
    em: f64,
    linhas: Arc<Vec<Value>>,
    bytes: usize,
}

#[derive(Default)]
struct Snapshots {
    mapa: IndexMap<String, Snapshot>,
    bytes: usize,
}

impl Snapshots {
    fn remover(&mut self, chave: &str) {
        if let Some(s) = self.mapa.shift_remove(chave) {
            self.bytes -= s.bytes;
        }
    }
}

/// Resposta do proxy da Gamma: instante, status e corpo.
type RespostaCacheada = (Instant, u16, Vec<u8>);

/// Caches em memória da Polymarket.
#[derive(Default)]
pub struct EstadoPoly {
    snapshots: Mutex<Snapshots>,
    geracoes: Mutex<HashMap<String, u64>>,
    proxy: Mutex<HashMap<String, RespostaCacheada>>,
    saldos: Mutex<HashMap<String, (Instant, f64)>>,
}

fn agora_ms() -> f64 {
    chrono::Utc::now().timestamp_millis() as f64
}

fn texto(v: Option<&Value>) -> String {
    if js::truthy(v) { js::string(v) } else { String::new() }
}

fn wallet_valida(w: &str) -> bool {
    w.len() == 42 && w.starts_with("0x") && w[2..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// Parâmetros da URL em ordem, como `req.query` do Express (repetidos viram "a,b").
pub fn consulta_ordenada(query: Option<&str>) -> IndexMap<String, String> {
    let mut m: IndexMap<String, String> = IndexMap::new();
    for (k, v) in url::form_urlencoded::parse(query.unwrap_or("").as_bytes()) {
        match m.get_mut(k.as_ref()) {
            Some(a) => *a = format!("{a},{v}"),
            None => {
                m.insert(k.into_owned(), v.into_owned());
            }
        }
    }
    m
}

/// `URLSearchParams.toString()`.
fn serializar(pares: &[(String, String)]) -> String {
    let mut s = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in pares {
        s.append_pair(k, v);
    }
    s.finish()
}

/// `URLSearchParams.set()`: substitui a primeira ocorrência e remove as demais.
fn definir(pares: &mut Vec<(String, String)>, chave: &str, valor: &str) {
    match pares.iter().position(|(k, _)| k == chave) {
        Some(i) => {
            pares[i].1 = valor.to_string();
            let mut j = 0;
            pares.retain(|(k, _)| {
                let manter = k != chave || j == i;
                j += 1;
                manter
            });
        }
        None => pares.push((chave.to_string(), valor.to_string())),
    }
}

// ---------------------------------------------------------------------------
// Data API v2
// ---------------------------------------------------------------------------

const ALIASES: &[(&str, &str)] = &[
    ("proxy_wallet", "proxyWallet"),
    ("condition_id", "conditionId"),
    ("token_id", "asset"),
    ("event_slug", "eventSlug"),
    ("event_id", "eventId"),
    ("outcome_index", "outcomeIndex"),
    ("opposite_outcome", "oppositeOutcome"),
    ("opposite_token_id", "oppositeAsset"),
    ("end_date", "endDate"),
    ("avg_price", "avgPrice"),
    ("current_size", "size"),
    ("current_price", "curPrice"),
    ("current_value", "currentValue"),
    ("total_size", "totalBought"),
    ("realized_pnl", "realizedPnl"),
    ("unrealized_pnl", "cashPnl"),
    ("percent_pnl", "percentPnl"),
    ("percent_realized_pnl", "percentRealizedPnl"),
    ("negative_risk", "negativeRisk"),
    ("transaction_hash", "transactionHash"),
    ("usdc_size", "usdcSize"),
    ("profile_image", "profileImage"),
    ("profile_image_optimized", "profileImageOptimized"),
];

/// `normalizePolyV2()`: nomes da v1 e validação da economia das posições.
fn normalizar(linha: &Value, endpoint: &str) -> Result<Value> {
    let Some(obj) = linha.as_object() else { bail!("Polymarket v2: linha inválida") };
    let mut saida = Map::new();
    for (k, v) in obj {
        let nome = ALIASES.iter().find(|(a, _)| a == k).map_or(k.as_str(), |(_, b)| b);
        saida.insert(nome.to_string(), v.clone());
    }
    if endpoint == "positions" || endpoint == "closed-positions" {
        if !js::truthy(obj.get("token_id")) || !js::truthy(obj.get("condition_id")) {
            bail!("Polymarket v2: posição sem identidade");
        }
        for k in ["current_size", "avg_price", "entry_cost_usdc", "total_size", "realized_pnl", "unrealized_pnl"] {
            let finito = obj.get(k).and_then(Value::as_number).and_then(|n| n.as_f64()).is_some_and(f64::is_finite);
            if !finito {
                bail!("Polymarket v2: economia da posição incompleta");
            }
        }
        // Em CLOSED, entry_cost é a base restante (~0), nunca a stake histórica.
        if endpoint == "positions" {
            saida.insert("initialValue".into(), obj["entry_cost_usdc"].clone());
        }
        let ultimo = obj.get("last_event_at").filter(|v| js::truthy(Some(v))).cloned().unwrap_or(json!(0));
        saida.insert("timestamp".into(), ultimo);
        saida.insert("_polyDataVersion".into(), json!(2));
        if saida.get("endDate").and_then(Value::as_str) == Some("1970-01-01") {
            saida.insert("endDate".into(), Value::Null);
        }
    }
    Ok(Value::Object(saida))
}

struct Pedido {
    caminho: &'static str,
    endpoint: String,
    pares: Vec<(String, String)>,
}

/// `requestSpec()`.
fn pedido(endpoint: &str, consulta: &IndexMap<String, String>, wallet: &str) -> Result<Pedido> {
    if !["positions", "closed-positions", "activity", "trades"].contains(&endpoint) {
        bail!("Consulta Polymarket não suportada");
    }
    let mapa: &[(&str, &str)] = &[
        ("market", "condition"),
        ("condition", "condition"),
        ("condition_id", "condition"),
        ("conditionId", "condition"),
        ("eventId", "event_id"),
        ("event_id", "event_id"),
        ("sortBy", "sort_by"),
        ("sort_by", "sort_by"),
        ("sortDirection", "sort_direction"),
        ("sort_direction", "sort_direction"),
        ("start", "start"),
        ("end", "end"),
        ("title", "title"),
        ("side", "side"),
        ("type", "type"),
        ("takerOnly", "taker_only"),
    ];
    let mut q: Vec<(String, String)> = vec![("user".into(), wallet.into())];
    for (k, v) in consulta {
        if let Some((_, destino)) = mapa.iter().find(|(a, _)| a == k) {
            definir(&mut q, destino, v);
        }
    }
    if let Some((_, c)) = q.iter().find(|(k, _)| k == "condition") {
        let distintos: std::collections::HashSet<&str> = c.split(',').filter(|s| !s.is_empty()).collect();
        if distintos.len() > 20 {
            bail!("Polymarket v2 aceita até 20 mercados por consulta");
        }
    }
    if endpoint == "positions" || endpoint == "closed-positions" {
        definir(&mut q, "status", if endpoint == "closed-positions" { "CLOSED" } else { "OPEN" });
        definir(&mut q, "filter_type", "TOKENS");
        definir(&mut q, "filter_amount", consulta.get("sizeThreshold").map_or("0", |s| s.as_str()));
        if endpoint == "positions" {
            definir(&mut q, "include_archived", "true");
        }
    } else if !q.iter().any(|(k, _)| k == "start") {
        definir(&mut q, "start", "1");
    }
    definir(&mut q, "limit", "1000");
    // `URLSearchParams.sort()`: estável, por chave.
    q.sort_by(|a, b| a.0.cmp(&b.0));
    let caminho = match endpoint {
        "closed-positions" | "positions" => "positions",
        "activity" => "activity",
        _ => "trades",
    };
    Ok(Pedido { caminho, endpoint: endpoint.to_string(), pares: q })
}

/// Percorre todas as páginas pelo cursor (página curta ou vazia não encerra).
async fn buscar_linhas(p: &Pedido, pares: &[(String, String)]) -> Result<Vec<Value>> {
    let mut linhas = Vec::new();
    let mut vistos = std::collections::HashSet::new();
    let mut cursor: Option<String> = None;
    let mut paginas = 0;
    loop {
        paginas += 1;
        if paginas > 10_000 {
            bail!("Polymarket v2: consulta sem fim confirmado");
        }
        let mut q = pares.to_vec();
        if let Some(c) = &cursor {
            definir(&mut q, "cursor", c);
        }
        let url = format!("{DATA_V2}/{}?{}", p.caminho, serializar(&q));
        let r = net::CLIENTE
            .get(&url)
            .header("accept", "application/json")
            .header("user-agent", "Betty/1.0")
            .timeout(Duration::from_secs(25))
            .send()
            .await?;
        if !r.status().is_success() {
            bail!("Polymarket v2 HTTP {}", r.status().as_u16());
        }
        let corpo: Value = serde_json::from_str(&r.text().await?)?;
        let pag = corpo.get("pagination");
        let tem_mais = pag.and_then(|p| p.get("has_more")).and_then(Value::as_bool);
        let proximo = pag.and_then(|p| p.get("next_cursor"));
        let valida = match tem_mais {
            Some(true) => proximo.and_then(Value::as_str).is_some_and(|s| !s.is_empty()),
            Some(false) => proximo.is_none_or(Value::is_null),
            None => false,
        };
        let Some(dados) = corpo.get("data").and_then(Value::as_array).filter(|_| valida) else {
            bail!("Polymarket v2: paginação inválida ou incompleta");
        };
        for l in dados {
            linhas.push(normalizar(l, &p.endpoint)?);
        }
        cursor = proximo.and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
        match &cursor {
            Some(c) if !vistos.insert(c.clone()) => bail!("Polymarket v2: cursor repetido"),
            Some(_) => {}
            None => return Ok(linhas),
        }
    }
}

fn ts(l: &Value) -> f64 {
    js::number(l.get("timestamp"))
}

fn inteiro_seguro(n: f64) -> bool {
    n.is_finite() && n.fract() == 0.0 && n.abs() <= 9_007_199_254_740_991.0
}

/// `validRows()`: timestamps inteiros, positivos, até `fim` e em ordem decrescente.
fn linhas_validas(linhas: &[Value], fim: f64) -> bool {
    linhas.iter().enumerate().all(|(i, l)| {
        let t = ts(l);
        inteiro_seguro(t) && t > 0.0 && t <= fim && (i == 0 || t <= ts(&linhas[i - 1]))
    })
}

fn sha256_hex(s: &str) -> String {
    hex::encode(ring::digest::digest(&ring::digest::SHA256, s.as_bytes()))
}

async fn atividade_carregar(db: &Db, user: &str, chave: &str) -> Option<Value> {
    let (u, k) = (user.to_string(), sha256_hex(chave));
    db.call(move |c| {
        Ok(c.query_row("SELECT data_json FROM polymarket_activity_cache WHERE user_id=? AND cache_key=?", [&u, &k], |r| r.get::<_, String>(0))
            .optional()?)
    })
    .await
    .ok()
    .flatten()
    .and_then(|t| serde_json::from_str(&t).ok())
}

async fn atividade_salvar(db: &Db, user: &str, chave: &str, valor: Value, em: f64) {
    let texto = js::para_texto(&valor);
    if texto.len() > MAX_ENTRADA_ATIVIDADE {
        return;
    }
    let (u, k) = (user.to_string(), sha256_hex(chave));
    // Falha do cache nunca invalida uma resposta completa da API.
    let _ = db
        .call(move |c| {
            let tx = c.transaction()?;
            tx.execute("DELETE FROM polymarket_activity_cache WHERE updated_at<?", [em - RECHECAGEM_ATIVIDADE_MS])?;
            tx.execute(
                "INSERT INTO polymarket_activity_cache VALUES (?,?,?,?)
    ON CONFLICT(user_id,cache_key) DO UPDATE SET data_json=excluded.data_json,updated_at=excluded.updated_at",
                rusqlite::params![u, k, texto, em],
            )?;
            // Limite do cache opcional, independente do histórico canônico.
            let entradas: Vec<(String, String, i64)> = {
                let mut s = tx.prepare(
                    "SELECT user_id,cache_key,length(CAST(data_json AS BLOB)) AS bytes FROM polymarket_activity_cache ORDER BY updated_at DESC",
                )?;
                s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?
            };
            let mut bytes = 0i64;
            for (i, (uu, kk, b)) in entradas.into_iter().enumerate() {
                bytes += b;
                if i >= 24 || bytes > 64 * 1024 * 1024 {
                    tx.execute("DELETE FROM polymarket_activity_cache WHERE user_id=? AND cache_key=?", [&uu, &kk])?;
                }
            }
            tx.commit()?;
            Ok(())
        })
        .await;
}

impl EstadoPoly {
    fn geracao(&self, user: &str) -> u64 {
        self.geracoes.lock().ok().and_then(|g| g.get(user).copied()).unwrap_or(0)
    }

    /// Guarda uma resposta do Gamma, sem passar de `MAX_PROXY_BYTES` no total:
    /// saem primeiro as vencidas e depois as mais antigas.
    fn guardar_proxy(&self, chave: String, status: u16, corpo: Vec<u8>) {
        if corpo.len() > MAX_PROXY_BYTES {
            return;
        }
        let Ok(mut p) = self.proxy.lock() else { return };
        p.remove(&chave);
        p.retain(|_, (em, _, _)| em.elapsed() < TTL_PROXY);
        let mut total: usize = p.values().map(|(_, _, c)| c.len()).sum();
        while total + corpo.len() > MAX_PROXY_BYTES {
            let Some(antiga) = p.iter().min_by_key(|(_, (em, _, _))| *em).map(|(k, _)| k.clone()) else { break };
            if let Some((_, _, c)) = p.remove(&antiga) {
                total -= c.len();
            }
        }
        p.insert(chave, (Instant::now(), status, corpo));
    }

    /// Tira o que já venceu e nunca mais seria devolvido: snapshots, respostas
    /// do Gamma e saldos.
    pub fn limpar_vencidos(&self) {
        if let Ok(mut s) = self.snapshots.lock() {
            let agora = agora_ms();
            let vencidas: Vec<String> = s.mapa.iter().filter(|(_, v)| agora - v.em >= TTL_SNAPSHOT_MS).map(|(k, _)| k.clone()).collect();
            for k in vencidas {
                s.remover(&k);
            }
        }
        if let Ok(mut p) = self.proxy.lock() {
            p.retain(|_, (em, _, _)| em.elapsed() < TTL_PROXY);
        }
        if let Ok(mut s) = self.saldos.lock() {
            s.retain(|_, (em, _)| em.elapsed() < TTL_SALDO);
        }
    }

    /// `clearUser()` + caches do proxy e do saldo.
    pub fn limpar_usuario(&self, user: &str) {
        if let Ok(mut g) = self.geracoes.lock() {
            *g.entry(user.to_string()).or_insert(0) += 1;
        }
        if let Ok(mut s) = self.snapshots.lock() {
            let chaves: Vec<String> = s.mapa.keys().filter(|k| k.starts_with(&format!("{user}|"))).cloned().collect();
            for k in chaves {
                s.remover(&k);
            }
        }
        if let Ok(mut p) = self.proxy.lock() {
            p.retain(|k, _| !k.starts_with(&format!("{user}|")));
        }
        if let Ok(mut s) = self.saldos.lock() {
            s.remove(user);
        }
    }
}

/// `readActivityIncremental()`: base completa relida a cada 10 minutos; entre
/// elas, só a janela recente (com 120 s de sobreposição) é substituída.
async fn atividade_incremental(
    estado: &Estado,
    user: &str,
    chave: &str,
    fim: f64,
    p: &Pedido,
    atual: &(dyn Fn() -> bool + Sync),
) -> Result<Vec<Value>> {
    let em = agora_ms();
    let antiga = atividade_carregar(&estado.db, user, chave).await;
    let n = |v: Option<&Value>| js::number(v);
    let utilizavel = antiga.as_ref().is_some_and(|a| {
        let full = n(a.get("fullAt"));
        let fim_antigo = n(a.get("end"));
        n(a.get("version")) == 1.0
            && a.get("version").is_some_and(Value::is_number)
            && inteiro_seguro(full)
            && full <= em
            && em - full < RECHECAGEM_ATIVIDADE_MS
            && inteiro_seguro(fim_antigo)
            && fim_antigo > 0.0
            && a.get("rows").and_then(Value::as_array).is_some_and(|l| linhas_validas(l, fim_antigo))
    });
    let verificar = || -> Result<()> { if atual() { Ok(()) } else { bail!("Polymarket v2: consulta obsoleta") } };
    let antigas: Vec<Value> = antiga.as_ref().and_then(|a| a.get("rows")).and_then(Value::as_array).cloned().unwrap_or_default();
    let fim_antigo = antiga.as_ref().map_or(0.0, |a| n(a.get("end")));
    if utilizavel && fim <= fim_antigo {
        verificar()?;
        return Ok(antigas.into_iter().filter(|l| ts(l) <= fim).collect());
    }
    let inicio = if utilizavel { (fim_antigo - 120.0).max(1.0) } else { 1.0 };
    let janela = |de: f64| {
        let mut q = p.pares.clone();
        definir(&mut q, "start", &js::numero_texto(de).unwrap_or_default());
        definir(&mut q, "end", &js::numero_texto(fim).unwrap_or_default());
        q
    };
    let mut recentes = buscar_linhas(p, &janela(inicio)).await?;
    verificar()?;
    let mut incremental = utilizavel;
    if !linhas_validas(&recentes, fim) || recentes.iter().any(|l| ts(l) < inicio) {
        if inicio > 1.0 {
            recentes = buscar_linhas(p, &janela(1.0)).await?;
            verificar()?;
        }
        incremental = false;
    }
    let linhas: Vec<Value> = if incremental {
        recentes.into_iter().chain(antigas.into_iter().filter(|l| ts(l) < inicio)).collect()
    } else {
        recentes
    };
    // Evidência temporal inválida nunca é gravada nem usada como base.
    if linhas_validas(&linhas, fim) && fim >= (em / 1000.0).floor() - 300.0 {
        let full = if incremental { antiga.as_ref().and_then(|a| a.get("fullAt")).cloned().unwrap_or(Value::Null) } else { js::numero_json(em) };
        let valor = json!({ "version": 1, "end": js::numero_json(fim), "fullAt": full, "rows": linhas });
        atividade_salvar(&estado.db, user, chave, valor, em).await;
    }
    Ok(linhas)
}

/// `readAll()`: snapshot de 60 s por conta, carteira e filtros (até 24 e 64 MB).
pub async fn ler_tudo(estado: &Estado, user: &str, wallet: &str, endpoint: &str, consulta: &IndexMap<String, String>) -> Result<Arc<Vec<Value>>> {
    let p = pedido(endpoint, consulta, wallet)?;
    let qs = serializar(&p.pares);
    let chave = format!("{user}|{}|{}|{qs}", wallet.to_lowercase(), p.caminho);
    let poly = &estado.poly;
    if let Ok(s) = poly.snapshots.lock() {
        if let Some(hit) = s.mapa.get(&chave).filter(|h| agora_ms() - h.em < TTL_SNAPSHOT_MS) {
            return Ok(hit.linhas.clone());
        }
    }
    let geracao = poly.geracao(user);
    let atual = || poly.geracao(user) == geracao;
    let get = |k: &str| p.pares.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str());
    let fim = get("end").map_or(f64::NAN, |v| js::number(Some(&json!(v))));
    let elegivel = endpoint == "activity"
        && get("start") == Some("1")
        && get("sort_by") == Some("TIMESTAMP")
        && get("sort_direction") == Some("DESC")
        && inteiro_seguro(fim)
        && fim > 0.0
        && p.pares.iter().all(|(k, _)| ["user", "start", "end", "sort_by", "sort_direction", "limit"].contains(&k.as_str()));
    let linhas = if elegivel {
        let duravel: Vec<(String, String)> = p.pares.iter().filter(|(k, _)| k != "end").cloned().collect();
        let chave_duravel = format!("{}|{}", wallet.to_lowercase(), serializar(&duravel));
        atividade_incremental(estado, user, &chave_duravel, fim, &p, &atual).await?
    } else {
        let linhas = buscar_linhas(&p, &p.pares).await?;
        if !atual() {
            bail!("Polymarket v2: consulta obsoleta");
        }
        linhas
    };
    if !atual() {
        bail!("Polymarket v2: consulta obsoleta");
    }
    let tamanho = js::para_vec(&linhas).len();
    let linhas = Arc::new(linhas);
    if let Ok(mut s) = poly.snapshots.lock() {
        s.remover(&chave);
        let agora = agora_ms();
        let vencidas: Vec<String> = s.mapa.iter().filter(|(_, v)| agora - v.em >= TTL_SNAPSHOT_MS).map(|(k, _)| k.clone()).collect();
        for k in vencidas {
            s.remover(&k);
        }
        while !s.mapa.is_empty() && (s.bytes + tamanho > MAX_SNAPSHOTS_BYTES || s.mapa.len() >= 24) {
            let primeira = s.mapa.keys().next().cloned().unwrap_or_default();
            s.remover(&primeira);
        }
        if tamanho <= MAX_SNAPSHOTS_BYTES {
            s.bytes += tamanho;
            s.mapa.insert(chave, Snapshot { em: agora, linhas: linhas.clone(), bytes: tamanho });
        }
    }
    Ok(linhas)
}

// ---------------------------------------------------------------------------
// Carteira, proxy e saldo
// ---------------------------------------------------------------------------

/// Carteira decifrada do usuário. `Ok(None)` quando não está conectada.
async fn carteira(estado: &Estado) -> Result<Option<String>> {
    let u = estado.usuario.id.clone();
    let enc: Option<String> = estado
        .db
        .call(move |c| {
            Ok(c.query_row("SELECT polymarket_wallet_enc FROM connections WHERE user_id=?", [&u], |r| r.get::<_, Option<String>>(0))
                .optional()?
                .flatten())
        })
        .await?;
    match enc.filter(|e| !e.is_empty()) {
        None => Ok(None),
        Some(e) => Ok(Some(estado.chave.decrypt(&e)?.to_string())),
    }
}

fn resposta(status: StatusCode, body: Value) -> ApiResponse {
    ApiResponse { status, body, bruto: None }
}

fn bytes_json(status: StatusCode, bytes: Vec<u8>) -> ApiResponse {
    ApiResponse { status, body: Value::Null, bruto: Some(bytes) }
}

fn decodificar_segmentos(resto: &str) -> String {
    resto
        .split('/')
        .filter(|s| !s.is_empty())
        .map(percent_decode)
        .collect::<Vec<_>>()
        .join("/")
}

fn percent_decode(s: &str) -> String {
    url::form_urlencoded::parse(format!("x={}", s.replace('+', "%2B")).as_bytes()).next().map_or_else(|| s.to_string(), |(_, v)| v.into_owned())
}

/// `GET /api/polymarket/*` e `GET /api/polymarket-gamma/*` (`proxyPolymarket()`).
pub async fn proxy(estado: &Estado, gamma: bool, resto: &str, query: Option<&str>) -> ApiResponse {
    let wallet = match carteira(estado).await {
        Ok(Some(w)) => w,
        Ok(None) => return resposta(StatusCode::CONFLICT, json!({ "ok": false, "error": "Conecte sua conta Polymarket primeiro." })),
        Err(e) => {
            eprintln!("[polymarket] carteira: {e:#}");
            return resposta(StatusCode::INTERNAL_SERVER_ERROR, json!({ "ok": false, "error": "Credencial Polymarket indisponível no servidor." }));
        }
    };
    let consulta = consulta_ordenada(query);
    let endpoint = decodificar_segmentos(resto);
    let user = estado.usuario.id.clone();
    if !gamma {
        let num = |k: &str| match consulta.get(k).filter(|v| !v.is_empty()) {
            Some(v) => js::number(Some(&json!(v))),
            None => f64::NAN,
        };
        let deslocamento = { let n = num("offset"); if n.is_nan() && !consulta.get("offset").is_some_and(|v| !v.is_empty()) { 0.0 } else { n } };
        let limite = { let n = num("limit"); if n.is_nan() && !consulta.get("limit").is_some_and(|v| !v.is_empty()) { 500.0 } else { n } };
        if !inteiro_seguro(deslocamento) || deslocamento < 0.0 || !inteiro_seguro(limite) || !(1.0..=1000.0).contains(&limite) {
            return resposta(StatusCode::BAD_REQUEST, json!({ "ok": false, "error": "Paginação inválida." }));
        }
        return match ler_tudo(estado, &user, &wallet, &endpoint, &consulta).await {
            Ok(linhas) => {
                let ini = (deslocamento as usize).min(linhas.len());
                let fim = (ini + limite as usize).min(linhas.len());
                bytes_json(StatusCode::OK, js::para_vec(&linhas[ini..fim]))
            }
            Err(e) => {
                eprintln!("[polymarket] v2: {e:#}");
                resposta(StatusCode::BAD_GATEWAY, json!({ "ok": false, "error": e.to_string() }))
            }
        };
    }
    let mut pares: Vec<(String, String)> = consulta.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    definir(&mut pares, "user", &wallet);
    let alvo = format!("{GAMMA}/{endpoint}?{}", serializar(&pares));
    let sem_cache: Vec<(String, String)> = consulta.iter().filter(|(k, _)| k.as_str() != "_").map(|(k, v)| (k.clone(), v.clone())).collect();
    let chave = format!("{user}|{}|{GAMMA}|{}|{}", wallet.to_lowercase(), endpoint.replace('/', ","), serializar(&sem_cache));
    if let Ok(p) = estado.poly.proxy.lock() {
        if let Some((em, status, corpo)) = p.get(&chave).filter(|(em, _, _)| em.elapsed() < TTL_PROXY) {
            let _ = em;
            return bytes_json(StatusCode::from_u16(*status).unwrap_or(StatusCode::OK), corpo.clone());
        }
    }
    let r = net::CLIENTE
        .get(&alvo)
        .header("accept", "application/json")
        .header("user-agent", "Betty/1.0")
        .timeout(Duration::from_secs(20))
        .send()
        .await;
    match r {
        Ok(r) => {
            let status = r.status().as_u16();
            match r.bytes().await {
                Ok(corpo) => {
                    let corpo = corpo.to_vec();
                    if (200..300).contains(&status) {
                        estado.poly.guardar_proxy(chave, status, corpo.clone());
                    }
                    bytes_json(StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY), corpo)
                }
                Err(_) => resposta(StatusCode::BAD_GATEWAY, json!({ "ok": false, "error": "Falha ao consultar Polymarket." })),
            }
        }
        Err(e) => {
            eprintln!("[polymarket] proxy: {e:#}");
            resposta(StatusCode::BAD_GATEWAY, json!({ "ok": false, "error": "Falha ao consultar Polymarket." }))
        }
    }
}

/// `fetchErc20Balance()`: `balanceOf` via `eth_call`, só com o endereço público.
pub(crate) async fn saldo_erc20(wallet: &str, rpc: &str, token: &str, decimais: i32, prazo: Duration) -> Result<f64> {
    if !wallet_valida(wallet) {
        bail!("Endereço da carteira inválido.");
    }
    let dados = format!("0x70a08231{:0>64}", wallet[2..].to_lowercase());
    let corpo = json!({ "jsonrpc": "2.0", "id": 1, "method": "eth_call", "params": [{ "to": token, "data": dados }, "latest"] });
    let cliente = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let r = cliente
        .post(rpc)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .body(corpo.to_string())
        .timeout(prazo)
        .send()
        .await?;
    if !r.status().is_success() {
        bail!("Consulta de saldo retornou HTTP {}.", r.status().as_u16());
    }
    let resposta: Value = serde_json::from_str(&r.text().await?)?;
    let resultado = texto(resposta.get("result"));
    let hex_valido = resultado.len() > 2 && resultado.len() <= 66 && resultado.starts_with("0x") && resultado[2..].bytes().all(|b| b.is_ascii_hexdigit());
    if js::truthy(resposta.get("error")) || !hex_valido {
        bail!("Resposta de saldo inválida.");
    }
    // `Number(BigInt(x))`: arredonda para o double mais próximo.
    let digitos = resultado[2..].trim_start_matches('0');
    let inteiro = if digitos.len() <= 32 {
        u128::from_str_radix(if digitos.is_empty() { "0" } else { digitos }, 16)? as f64
    } else {
        digitos.chars().fold(0f64, |a, c| a * 16.0 + c.to_digit(16).unwrap_or(0) as f64)
    };
    Ok(inteiro / 10f64.powi(decimais))
}

pub async fn saldo_pusd(wallet: &str) -> Result<f64> {
    rustls_pronto();
    saldo_erc20(wallet, POLYGON_RPC, PUSD, 6, Duration::from_secs(20)).await
}

/// Garante o provedor criptográfico do rustls também em clientes avulsos.
pub(crate) fn rustls_pronto() {
    let _ = &*net::CLIENTE;
}

/// `GET /api/polymarket/balance` (cache de 30 s).
pub async fn saldo(estado: &Estado) -> ApiResponse {
    let user = estado.usuario.id.clone();
    if let Ok(s) = estado.poly.saldos.lock() {
        if let Some((_, v)) = s.get(&user).filter(|(em, _)| em.elapsed() < TTL_SALDO) {
            return resposta(StatusCode::OK, json!({ "ok": true, "balance": js::numero_json(*v) }));
        }
    }
    let resultado: Result<f64> = async {
        let w = carteira(estado).await?.ok_or_else(|| anyhow!("Conecte sua conta Polymarket primeiro."))?;
        saldo_pusd(&w).await
    }
    .await;
    match resultado {
        Ok(v) => {
            if let Ok(mut s) = estado.poly.saldos.lock() {
                s.insert(user, (Instant::now(), v));
            }
            resposta(StatusCode::OK, json!({ "ok": true, "balance": js::numero_json(v) }))
        }
        Err(e) => {
            eprintln!("[polymarket] saldo: {e:#}");
            resposta(StatusCode::BAD_GATEWAY, json!({ "ok": false, "error": "Não foi possível consultar o saldo pUSD da Polymarket." }))
        }
    }
}

/// `resolvePolymarketTradingWallet()`: o login aponta para a proxy wallet, onde ficam fundos e posições.
async fn wallet_de_negociacao(endereco: &str) -> String {
    let url = format!("{GAMMA}/public-profile?address={}", net::codificar(endereco));
    let r = net::CLIENTE
        .get(url)
        .header("accept", "application/json")
        .header("user-agent", "Betty/1.0")
        .timeout(Duration::from_secs(8))
        .send()
        .await;
    if let Ok(r) = r {
        if r.status().is_success() {
            if let Ok(t) = r.text().await {
                if let Ok(d) = serde_json::from_str::<Value>(&t) {
                    let proxy = texto(d.get("proxyWallet")).trim().to_string();
                    if wallet_valida(&proxy) {
                        return proxy;
                    }
                }
            }
        }
    }
    endereco.to_string()
}

fn mascarar(w: &str) -> Option<String> {
    wallet_valida(w).then(|| format!("{}…{}", &w[..6], &w[w.len() - 4..]))
}

/// `clearPolymarketHistory()`: trocar de carteira apaga o histórico da anterior.
async fn limpar_historico(estado: &Estado) -> Result<()> {
    let u = estado.usuario.id.clone();
    estado
        .db
        .call(move |c| {
            let tx = c.transaction()?;
            tx.execute("DELETE FROM closed_bets_index WHERE user_id=? AND platform='polymarket'", [&u])?;
            tx.execute("DELETE FROM closed_bets_ledger WHERE user_id=? AND platform='polymarket'", [&u])?;
            tx.commit()?;
            Ok(())
        })
        .await?;
    limpar_caches(estado).await;
    Ok(())
}

/// `clearPolymarketUserCaches()`.
async fn limpar_caches(estado: &Estado) {
    estado.poly.limpar_usuario(&estado.usuario.id);
    let u = estado.usuario.id.clone();
    let _ = estado.db.call(move |c| Ok(c.execute("DELETE FROM polymarket_activity_cache WHERE user_id=?", [&u])?)).await;
}

/// `POST /api/connections/polymarket`.
pub async fn conectar(estado: &Estado, corpo: &Value) -> Result<ApiResponse> {
    let wallet = texto(corpo.get("wallet")).trim().to_string();
    if !wallet_valida(&wallet) {
        return Ok(resposta(StatusCode::BAD_REQUEST, json!({ "ok": false, "error": "Wallet Polymarket inválida." })));
    }
    let negociacao = wallet_de_negociacao(&wallet).await;
    let anterior = carteira(estado).await.ok().flatten().unwrap_or_default();
    if anterior.to_lowercase() != negociacao.to_lowercase() {
        limpar_historico(estado).await?;
    }
    let cifrada = estado.chave.encrypt(&negociacao)?;
    let u = estado.usuario.id.clone();
    estado
        .db
        .call(move |c| {
            c.execute(
                "INSERT INTO connections(user_id,polymarket_wallet_enc,updated_at) VALUES(?,?,?) ON CONFLICT(user_id) DO UPDATE SET polymarket_wallet_enc=excluded.polymarket_wallet_enc,updated_at=excluded.updated_at",
                rusqlite::params![u, cifrada, agora()],
            )?;
            // Versões antigas podiam ter uma Private Key da Polymarket: nunca é mantida.
            let _ = c.execute("UPDATE connections SET polymarket_private_key_enc=NULL WHERE user_id=?", [&u]);
            Ok(())
        })
        .await?;
    limpar_caches(estado).await;
    Ok(resposta(StatusCode::OK, json!({ "ok": true })))
}

/// `DELETE /api/connections/polymarket`.
pub async fn desconectar(estado: &Estado) -> Result<ApiResponse> {
    let u = estado.usuario.id.clone();
    estado
        .db
        .call(move |c| {
            let t = agora();
            if c.execute(
                "UPDATE connections SET polymarket_wallet=NULL,polymarket_wallet_enc=NULL,polymarket_private_key_enc=NULL,updated_at=? WHERE user_id=?",
                rusqlite::params![t, u],
            )
            .is_err()
            {
                c.execute(
                    "UPDATE connections SET polymarket_wallet=NULL,polymarket_wallet_enc=NULL,updated_at=? WHERE user_id=?",
                    rusqlite::params![t, u],
                )?;
            }
            Ok(())
        })
        .await?;
    limpar_historico(estado).await?;
    Ok(resposta(StatusCode::OK, json!({ "ok": true })))
}

/// `POST /api/connections/polymarket/preview`: confere a carteira antes de salvar.
pub async fn preview(estado: &Estado, corpo: &Value) -> ApiResponse {
    let wallet = texto(corpo.get("wallet")).trim().to_string();
    if !wallet_valida(&wallet) {
        return resposta(StatusCode::BAD_REQUEST, json!({ "ok": false, "error": "Wallet Polymarket inválida." }));
    }
    let resultado: Result<Value> = async {
        let proxy_wallet = wallet_de_negociacao(&wallet).await;
        let vazio = IndexMap::new();
        let (saldo, abertas, fechadas) = futures::join!(
            saldo_pusd(&proxy_wallet),
            ler_tudo(estado, "preview", &proxy_wallet, "positions", &vazio),
            ler_tudo(estado, "preview", &proxy_wallet, "closed-positions", &vazio)
        );
        let (saldo, abertas, fechadas) = (saldo?, abertas?.len(), fechadas?.len());
        let vazia = saldo <= 0.0 && abertas == 0 && fechadas == 0;
        Ok(json!({
            "ok": true,
            "inputWallet": wallet,
            "proxyWallet": proxy_wallet,
            "proxyWalletHint": mascarar(&proxy_wallet),
            "resolvedFromLogin": wallet.to_lowercase() != proxy_wallet.to_lowercase(),
            "balance": js::numero_json(saldo),
            "openPositions": abertas,
            "openPositionsCapped": false,
            "closedSample": fechadas,
            "closedSampleCapped": false,
            "empty": vazia,
            "warning": if vazia {
                json!("Esta carteira parece vazia: saldo zero e nenhuma posição. Confira se o endereço é o de login da Polymarket — o Betty usa a proxy wallet, onde ficam fundos e posições.")
            } else {
                Value::Null
            }
        }))
    }
    .await;
    resultado.map_or_else(
        |e| {
            eprintln!("[polymarket] preview: {e:#}");
            resposta(StatusCode::BAD_GATEWAY, json!({ "ok": false, "error": "Não foi possível validar a wallet da Polymarket." }))
        },
        |v| resposta(StatusCode::OK, v),
    )
}

// ---------------------------------------------------------------------------
// Cache de metadados públicos (polymarket_metadata_cache)
// ---------------------------------------------------------------------------

/// `validMetadata()`.
fn metadado_valido(e: &Value) -> bool {
    let slug_ok = e.get("slug").and_then(Value::as_str).is_some_and(|s| !s.is_empty() && s.encode_utf16().count() <= 500);
    let Some(d) = e.get("data").filter(|d| js::truthy(Some(d))) else { return false };
    let inicio_ok = d.get("start").and_then(Value::as_str).is_some_and(|s| js::data_ms(s).is_some());
    let datas = d.get("dates");
    let mercados = d.get("sport").and_then(|s| s.get("markets")).and_then(Value::as_array);
    slug_ok
        && inicio_ok
        && js::truthy(datas.and_then(|x| x.get("event")))
        && js::truthy(datas.and_then(|x| x.get("markets")))
        && mercados.is_some_and(|m| {
            !m.is_empty() && m.iter().any(|x| x.get("tokenIds").and_then(Value::as_array).is_some_and(|t| !t.is_empty()))
        })
}

/// Mercados de um evento que uma consulta do cache precisa: os das apostas que
/// pediram (`need` da interface) e os de todas as apostas da conta naquele
/// evento que estão no ledger.
#[derive(Default)]
struct Necessidade {
    ativos: HashSet<String>,
    condicoes: HashSet<String>,
    slugs: HashSet<String>,
}

/// Forma frouxa de comparar identificadores: sem espaços nem aspas e em
/// minúsculas. Tudo o que o `exactMarket()` (com `clean`) e o `selectedMarket`
/// dos títulos (com `trim`) consideram igual também é igual aqui, então o
/// recorte nunca deixa de fora um mercado que eles achariam.
fn frouxo(v: Option<&Value>) -> String {
    let texto = match v {
        None | Some(Value::Null) => return String::new(),
        Some(Value::String(s)) => s.clone(),
        outro => js::string(outro),
    };
    texto.chars().filter(|c| !c.is_whitespace() && *c != '"').flat_map(char::to_lowercase).collect()
}

impl Necessidade {
    /// Junta os identificadores de uma aposta (`asset`, `conditionId`, `slug`...)
    /// ou de um pedido da interface (listas `assets`, `conditions` e `slugs`).
    fn juntar(&mut self, d: &Value) {
        let campos: [(&[&str], &str, &mut HashSet<String>); 3] = [
            (&["asset", "assetId", "tokenId"], "assets", &mut self.ativos),
            (&["conditionId", "condition_id"], "conditions", &mut self.condicoes),
            (&["slug"], "slugs", &mut self.slugs),
        ];
        for (chaves, lista, conjunto) in campos {
            let soltos = chaves.iter().map(|k| d.get(*k));
            let da_lista = d.get(lista).and_then(Value::as_array).into_iter().flatten().map(Some);
            for v in soltos.chain(da_lista) {
                let v = frouxo(v);
                if !v.is_empty() {
                    conjunto.insert(v);
                }
            }
        }
    }

    fn mercado(&self, m: &Value) -> bool {
        m.get("tokenIds").and_then(Value::as_array).is_some_and(|ids| ids.iter().any(|t| self.ativos.contains(&frouxo(Some(t)))))
            || ["conditionId", "condition_id"].iter().any(|k| self.condicoes.contains(&frouxo(m.get(*k))))
            || self.slugs.contains(&frouxo(m.get("slug")))
    }
}

/// Reduz o metadado de um evento aos mercados que alguém da conta usa.
///
/// - Datas: o evento inteiro e, dos mercados, só os das apostas. As datas são
///   lidas por `markets[slug da aposta]`.
/// - Esporte: `texts`, tipos e o resto do evento ficam; dos mercados, só os
///   que casam com alguma aposta. `eventTexts` vai calculado com a lista
///   inteira, como `sinaisDaAposta()` faria. Se nenhum mercado mantido tiver
///   `tokenIds`, o primeiro que tiver fica junto, porque a interface exige um.
///
/// Com algum mercado que não é objeto (o JS lançaria exceção), o esporte não é
/// recortado.
fn recortar_evento(dado: &mut Value, n: &Necessidade) {
    if let Some(Value::Object(mercados)) = dado.pointer_mut("/dates/markets") {
        mercados.retain(|k, _| n.slugs.contains(&frouxo(Some(&Value::String(k.clone())))));
    }
    let Some(esporte) = dado.get_mut("sport").and_then(Value::as_object_mut) else { return };
    let Some(Value::Array(lista)) = esporte.get("markets") else { return };
    if !lista.iter().all(Value::is_object) {
        return;
    }
    let textos_evento = match esporte.get("eventTexts") {
        Some(Value::Array(_)) => None,
        _ => {
            let textos_mercado: HashSet<String> = lista
                .iter()
                .flat_map(|m| ["slug", "question", "groupItemTitle", "sportsMarketType"].map(|k| m.get(k)))
                .filter(|v| js::truthy(*v))
                .map(|v| js::string(v).to_lowercase())
                .collect();
            let lista_textos = match esporte.get("texts") {
                Some(Value::Array(a)) => {
                    a.iter().filter(|v| !textos_mercado.contains(&js::string(Some(v)).to_lowercase())).cloned().collect()
                }
                _ => Vec::new(),
            };
            Some(Value::Array(lista_textos))
        }
    };
    let tem_token = |m: &Value| m.get("tokenIds").and_then(Value::as_array).is_some_and(|t| !t.is_empty());
    let mut manter: Vec<bool> = lista.iter().map(|m| n.mercado(m)).collect();
    if !lista.iter().zip(&manter).any(|(m, &k)| k && tem_token(m))
        && let Some(i) = lista.iter().position(tem_token)
    {
        manter[i] = true;
    }
    let novos: Vec<Value> = lista.iter().zip(manter).filter(|(_, k)| *k).map(|(m, _)| m.clone()).collect();
    esporte.insert("markets".into(), Value::Array(novos));
    if let Some(t) = textos_evento {
        esporte.insert("eventTexts".into(), t);
    }
}

/// Mercados que cada evento pedido precisa: os do pedido mais os das apostas do
/// ledger que ainda não têm data de resultado confirmada (`_confirmedClosedTs`).
/// Para essas, a data do PnL na interface pode vir do mapa de datas da última
/// consulta (`metasDatasPnl`), lido pelo mercado da própria aposta.
fn necessidades(c: &rusqlite::Connection, user: &str, pedido: Option<&Value>, chaves: &[String]) -> Result<HashMap<String, Necessidade>> {
    let Some(pedido) = pedido.and_then(Value::as_object).filter(|p| !p.is_empty()) else { return Ok(HashMap::new()) };
    let mut mapa: HashMap<String, Necessidade> = HashMap::new();
    for slug in chaves {
        if let Some(n) = pedido.get(slug) {
            mapa.entry(slug.clone()).or_default().juntar(n);
        }
    }
    if mapa.is_empty() {
        return Ok(mapa);
    }
    let mut s = c.prepare(
        "SELECT json_extract(data_json,'$.eventSlug'),json_extract(data_json,'$.slug'),json_extract(data_json,'$.asset'),
                json_extract(data_json,'$.conditionId') FROM closed_bets_ledger WHERE user_id=? AND platform='polymarket'
         AND json_extract(data_json,'$._confirmedClosedTs') IS NULL",
    )?;
    let texto = |r: &rusqlite::Row<'_>, i: usize| -> Value {
        match r.get_ref(i) {
            Ok(rusqlite::types::ValueRef::Text(t)) => Value::String(String::from_utf8_lossy(t).into_owned()),
            Ok(rusqlite::types::ValueRef::Integer(n)) => json!(n),
            _ => Value::Null,
        }
    };
    let linhas = s.query_map([user], |r| Ok((texto(r, 0), texto(r, 1), texto(r, 2), texto(r, 3))))?;
    for linha in linhas {
        let (evento, slug, ativo, condicao) = linha?;
        // `d.eventSlug || d.slug`, como a interface agrupa.
        let chave = if js::truthy(Some(&evento)) { js::string(Some(&evento)) } else { js::string(Some(&slug)) };
        if let Some(n) = mapa.get_mut(&chave) {
            n.juntar(&json!({ "slug": slug, "asset": ativo, "conditionId": condicao }));
        }
    }
    Ok(mapa)
}

/// `POST /api/polymarket-metadata/cache`: grava (entries) ou lê (slugs). Na
/// leitura, `need` diz de que apostas a interface precisa em cada evento e a
/// resposta vem recortada nesses mercados.
pub async fn cache_metadados(estado: &Estado, corpo: &Value) -> ApiResponse {
    let tamanho = |k: &str| -> usize {
        match corpo.get(k) {
            Some(Value::Array(a)) => a.len(),
            Some(Value::String(s)) => s.encode_utf16().count(),
            _ => 0,
        }
    };
    let n = if tamanho("entries") > 0 { tamanho("entries") } else { tamanho("slugs") };
    if n > 400 {
        return resposta(StatusCode::BAD_REQUEST, json!({ "ok": false, "error": "Envie metadados em lotes de até 400 eventos." }));
    }
    let u = estado.usuario.id.clone();
    let corpo = corpo.clone();
    let r = estado
        .db
        .call(move |c| {
            let agora = agora_ms();
            if let Some(entradas) = corpo.get("entries").and_then(Value::as_array) {
                let validas: Vec<&Value> = entradas.iter().filter(|e| metadado_valido(e)).collect();
                let tx = c.transaction()?;
                {
                    let mut s = tx.prepare(
                        "INSERT INTO polymarket_metadata_cache(user_id,slug,version,data_json,updated_at) VALUES(?,?,?,?,?)
    ON CONFLICT(user_id,slug) DO UPDATE SET version=excluded.version,data_json=excluded.data_json,updated_at=excluded.updated_at",
                    )?;
                    for e in &validas {
                        s.execute(rusqlite::params![u, e["slug"].as_str(), META_VERSAO, js::para_texto(&e["data"]), agora])?;
                    }
                }
                tx.commit()?;
                return Ok(json!({ "ok": true, "saved": validas.len() }));
            }
            let mut chaves: Vec<String> = Vec::new();
            for s in corpo.get("slugs").and_then(Value::as_array).map_or(&[][..], |a| a.as_slice()) {
                if let Some(s) = s.as_str().filter(|s| s.encode_utf16().count() <= 500) {
                    if !chaves.iter().any(|c| c == s) {
                        chaves.push(s.to_string());
                    }
                }
            }
            let precisa = necessidades(c, &u, corpo.get("need"), &chaves)?;
            let mut entradas = Vec::new();
            for lote in chaves.chunks(400) {
                let marcas = vec!["?"; lote.len()].join(",");
                let sql = format!(
                    "SELECT slug,data_json,updated_at FROM polymarket_metadata_cache
      WHERE user_id=? AND version=? AND updated_at>? AND updated_at<=? AND slug IN ({marcas})"
                );
                let mut s = c.prepare(&sql)?;
                let mut p: Vec<&dyn rusqlite::types::ToSql> = vec![&u, &META_VERSAO];
                let (de, ate) = (agora - META_TTL_MS, agora);
                p.push(&de);
                p.push(&ate);
                p.extend(lote.iter().map(|k| k as &dyn rusqlite::types::ToSql));
                for row in s.query_map(p.as_slice(), |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
                    let (slug, d) = row?;
                    if let Ok(dado) = serde_json::from_str::<Value>(&d) {
                        let mut e = json!({ "slug": slug, "data": dado });
                        if metadado_valido(&e) {
                            if let Some(n) = precisa.get(&slug) {
                                recortar_evento(&mut e["data"], n);
                            }
                            entradas.push(e);
                        }
                    }
                }
            }
            Ok(json!({ "ok": true, "entries": entradas }))
        })
        .await;
    r.map_or_else(
        |e| {
            eprintln!("[polymarket] cache de metadados: {e:#}");
            resposta(StatusCode::SERVICE_UNAVAILABLE, json!({ "ok": false }))
        },
        |v| resposta(StatusCode::OK, v),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evento() -> Value {
        json!({
            "start": "2026-09-20T18:00:00Z",
            "dates": { "event": { "startTime": "2026-09-20T18:00:00Z" }, "markets": {
                "jogo-ml": { "closedTime": "2026-09-20T20:00:00Z" },
                "jogo-total": { "closedTime": "2026-09-20T20:01:00Z" },
                "jogo-handicap": { "closedTime": "2026-09-20T20:02:00Z" }
            } },
            "sport": {
                "texts": ["A vs B", "Soccer", "jogo-ml", "Will A win?", "jogo-total", "O/U 2.5"],
                "sportsMarketTypes": ["moneyline", "totals"],
                "markets": [
                    { "conditionId": "0xC1", "slug": "jogo-ml", "question": "Will A win?", "tokenIds": ["11", "12"] },
                    { "conditionId": "0xC2", "slug": "jogo-total", "question": "O/U 2.5", "tokenIds": ["21", "22"] },
                    { "conditionId": "0xC3", "slug": "jogo-handicap", "question": "Handicap", "tokenIds": ["31", "32"] }
                ],
                "eventId": "9"
            }
        })
    }

    fn necessidade(d: Value) -> Necessidade {
        let mut n = Necessidade::default();
        n.juntar(&d);
        n
    }

    #[test]
    fn recorte_mantem_so_os_mercados_das_apostas() {
        let mut e = evento();
        recortar_evento(&mut e, &necessidade(json!({ "assets": [" 22 "], "conditions": [], "slugs": ["jogo-total"] })));
        assert_eq!(e["dates"]["markets"], json!({ "jogo-total": { "closedTime": "2026-09-20T20:01:00Z" } }));
        assert_eq!(e["dates"]["event"], evento()["dates"]["event"], "as datas do evento ficam");
        let mercados = e["sport"]["markets"].as_array().unwrap();
        assert_eq!(mercados.len(), 1);
        assert_eq!(mercados[0]["conditionId"], "0xC2");
        assert_eq!(e["sport"]["texts"], evento()["sport"]["texts"], "texts não muda");
        assert_eq!(e["sport"]["eventTexts"], json!(["A vs B", "Soccer"]), "calculado com todos os mercados");
        assert_eq!(e["sport"]["sportsMarketTypes"], json!(["moneyline", "totals"]));
        assert_eq!(e["start"], "2026-09-20T18:00:00Z");
    }

    #[test]
    fn recorte_guarda_um_mercado_com_tokens_e_respeita_mercado_invalido() {
        // A aposta casa só pelo slug com um mercado sem tokenIds.
        let mut e = evento();
        e["sport"]["markets"][2]["tokenIds"] = json!([]);
        recortar_evento(&mut e, &necessidade(json!({ "slugs": ["JOGO-HANDICAP"] })));
        let slugs: Vec<&str> = e["sport"]["markets"].as_array().unwrap().iter().map(|m| m["slug"].as_str().unwrap()).collect();
        assert_eq!(slugs, ["jogo-ml", "jogo-handicap"], "o primeiro mercado com tokenIds vai junto, na ordem original");

        let mut e = evento();
        e["sport"]["markets"].as_array_mut().unwrap().push(Value::Null);
        recortar_evento(&mut e, &necessidade(json!({ "slugs": ["jogo-ml"] })));
        assert_eq!(e["sport"]["markets"].as_array().unwrap().len(), 4, "com mercado nulo o esporte fica inteiro");
        assert_eq!(e["dates"]["markets"].as_object().unwrap().len(), 1);
    }

    #[test]
    fn apostas_do_ledger_sem_data_confirmada_entram_no_recorte() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = crate::db::abrir(&dir.path().join("t.db")).unwrap();
        let u = crate::db::usuario_local(&mut c).unwrap().id;
        let t = agora();
        for (chave, dado) in [
            ("k1", json!({ "eventSlug": "jogo", "slug": "jogo-handicap", "asset": "31", "conditionId": "0xC3" })),
            ("k2", json!({ "eventSlug": "jogo", "slug": "jogo-ml", "asset": "11", "conditionId": "0xC1", "_confirmedClosedTs": 1 })),
            ("k3", json!({ "eventSlug": "outro", "slug": "outro-ml", "asset": "99" })),
        ] {
            c.execute(
                "INSERT INTO closed_bets_ledger(user_id,platform,bet_key,data_json,first_seen_at,updated_at) VALUES(?,'polymarket',?,?,?,?)",
                rusqlite::params![u, chave, dado.to_string(), t, t],
            )
            .unwrap();
        }
        let pedido = json!({ "jogo": { "slugs": ["jogo-total"] } });
        let mapa = necessidades(&c, &u, Some(&pedido), &["jogo".to_string(), "outro".to_string()]).unwrap();
        let mut e = evento();
        recortar_evento(&mut e, &mapa["jogo"]);
        let mut slugs: Vec<&String> = e["dates"]["markets"].as_object().unwrap().keys().collect();
        slugs.sort();
        assert_eq!(slugs, ["jogo-handicap", "jogo-total"], "a do pedido e a do ledger sem data confirmada");
        assert!(!mapa.contains_key("outro"), "evento sem pedido não é recortado");
        assert!(necessidades(&c, &u, None, &["jogo".to_string()]).unwrap().is_empty(), "sem `need`, resposta inteira");
    }

    #[test]
    fn caches_vencidos_saem_e_o_gamma_tem_teto() {
        let p = EstadoPoly::default();
        let ha = |s: u64| Instant::now().checked_sub(Duration::from_secs(s)).unwrap();
        let mb = |n: usize| vec![b'x'; n * 1024 * 1024];
        p.guardar_proxy("u|a".into(), 200, mb(7));
        p.guardar_proxy("u|b".into(), 200, mb(7));
        p.guardar_proxy("u|c".into(), 200, mb(7));
        {
            let mapa = p.proxy.lock().unwrap();
            assert!(!mapa.contains_key("u|a"), "a mais antiga sai para caber no teto");
            assert!(mapa.contains_key("u|b") && mapa.contains_key("u|c"));
        }
        p.guardar_proxy("u|enorme".into(), 200, mb(17));
        assert!(!p.proxy.lock().unwrap().contains_key("u|enorme"), "maior que o teto não é guardada");

        p.proxy.lock().unwrap().get_mut("u|b").unwrap().0 = ha(46);
        p.saldos.lock().unwrap().insert("u".into(), (ha(31), 1.0));
        p.saldos.lock().unwrap().insert("v".into(), (Instant::now(), 2.0));
        {
            let mut s = p.snapshots.lock().unwrap();
            s.mapa.insert("velho".into(), Snapshot { em: agora_ms() - 61_000.0, linhas: Arc::new(vec![]), bytes: 10 });
            s.mapa.insert("novo".into(), Snapshot { em: agora_ms(), linhas: Arc::new(vec![]), bytes: 5 });
            s.bytes = 15;
        }
        p.limpar_vencidos();
        assert_eq!(p.proxy.lock().unwrap().keys().collect::<Vec<_>>(), vec!["u|c"]);
        assert_eq!(p.saldos.lock().unwrap().keys().collect::<Vec<_>>(), vec!["v"]);
        let s = p.snapshots.lock().unwrap();
        assert_eq!(s.mapa.keys().collect::<Vec<_>>(), vec!["novo"]);
        assert_eq!(s.bytes, 5, "a conta de bytes acompanha a remoção");
    }

    #[test]
    fn pedido_v2_igual_ao_js() {
        let mut c = IndexMap::new();
        c.insert("limit".to_string(), "500".to_string());
        c.insert("sortBy".to_string(), "TIMESTAMP".to_string());
        c.insert("market".to_string(), "0xa,0xb".to_string());
        c.insert("sizeThreshold".to_string(), "0".to_string());
        let p = pedido("positions", &c, "0xW").unwrap();
        assert_eq!(
            serializar(&p.pares),
            "condition=0xa%2C0xb&filter_amount=0&filter_type=TOKENS&include_archived=true&limit=1000&sort_by=TIMESTAMP&status=OPEN&user=0xW"
        );
        let a = pedido("activity", &IndexMap::new(), "0xW").unwrap();
        assert_eq!(serializar(&a.pares), "limit=1000&start=1&user=0xW");
        assert!(pedido("outro", &IndexMap::new(), "0xW").is_err());
    }

    #[test]
    fn normaliza_posicao() {
        let l = json!({"token_id": "1", "condition_id": "0xc", "current_size": 2, "avg_price": 0.5, "entry_cost_usdc": 1,
            "total_size": 2, "realized_pnl": 0, "unrealized_pnl": 0.1, "end_date": "1970-01-01", "last_event_at": 1790000000});
        let n = normalizar(&l, "positions").unwrap();
        assert_eq!(n["asset"], "1");
        assert_eq!(n["initialValue"], 1);
        assert_eq!(n["timestamp"], 1790000000);
        assert_eq!(n["endDate"], Value::Null);
        assert!(normalizar(&json!({"token_id": "1"}), "positions").is_err());
    }
}
