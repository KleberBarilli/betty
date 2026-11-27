//! Casas acompanhadas só por leitura: Predict.fun e SX Bet. Igual a
//! `lib/tracked-platforms.js` e às rotas `/api/connections/:platform`,
//! `/api/tracked/:platform/sync` e dos saldos no `server.js`.
//!
//! As chaves de API só leem posições, histórico e saldo; nada aqui envia
//! ordens. Diferença deliberada: o Node compartilhava a mesma promessa entre
//! duas sincronizações simultâneas da mesma casa; aqui cada pedido consulta a
//! plataforma (o resultado gravado é o mesmo).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use futures::StreamExt;
use indexmap::{IndexMap, IndexSet};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Map, Value, json};
use tauri::http::StatusCode;

use crate::api::{ApiResponse, Estado};
use crate::history::{odd_historica, stake_historico, ymd_sao_paulo};
use crate::{audit, js, ledger, net, poly, regex_js, titles};

const PREDICT: &str = "https://api.predict.fun/v1";
const SX: &str = "https://api.sx.bet";
const PREDICT_USDT: &str = "0x55d398326f99059fF775485246999027B3197955";
const BNB_RPCS: [&str; 2] = ["https://bsc-dataseed.bnbchain.org", "https://bsc-dataseed-public.bnbchain.org"];
const PRAZO_PADRAO: Duration = Duration::from_secs(20);
const TOLERANCIA: f64 = 1e-7;

/// `trackedPlatforms`.
pub fn casa_valida(casa: &str) -> bool {
    casa == "predict" || casa == "sxbet"
}

/// Agendas de categoria do Predict.fun (`predictScheduleCache`).
#[derive(Default)]
pub struct EstadoTracked {
    agendas: Mutex<IndexMap<String, Arc<Agenda>>>,
}

struct Agenda {
    titulo: String,
    escopo: String,
    mercados: IndexMap<String, MetaMercado>,
    expira: f64,
}

struct MetaMercado {
    data: String,
    categoria: &'static str,
    mercado: Value,
}

// ---------------------------------------------------------------------------
// HTTP e validação
// ---------------------------------------------------------------------------

/// `redirect: 'error'`: um redirecionamento vira resposta não-2xx (erro).
static CLIENTE: LazyLock<reqwest::Client> = LazyLock::new(|| {
    poly::rustls_pronto();
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("cliente HTTP")
});

fn invalida() -> anyhow::Error {
    anyhow!("Resposta inválida da plataforma.")
}

fn agora_ms() -> f64 {
    chrono::Utc::now().timestamp_millis() as f64
}

/// `application/x-www-form-urlencoded` do `URLSearchParams`.
fn form(s: &str) -> String {
    let mut saida = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b' ' => saida.push('+'),
            _ if b.is_ascii_alphanumeric() || b"*-._".contains(&b) => saida.push(b as char),
            _ => saida.push_str(&format!("%{b:02X}")),
        }
    }
    saida
}

type Consulta<'a> = Vec<(&'a str, String)>;

/// `request()`: GET com `accept: application/json`; não-2xx vira erro com o status.
async fn pedir(base: &str, caminho: &str, consulta: &Consulta<'_>, cabecalho: Option<(&str, &str)>, prazo: Duration) -> Result<Value> {
    let q: Vec<String> = consulta.iter().map(|(k, v)| format!("{}={}", form(k), form(v))).collect();
    let url = format!("{base}{caminho}?{}", q.join("&"));
    let mut r = CLIENTE.get(url).header("accept", "application/json").timeout(prazo);
    if let Some((k, v)) = cabecalho {
        r = r.header(k, v);
    }
    let resposta = r.send().await?;
    let status = resposta.status();
    if !status.is_success() {
        return Err(net::ErroHttp { status: status.as_u16(), corpo: Value::Null }.into());
    }
    Ok(serde_json::from_str(&resposta.text().await?)?)
}

/// `number()`: texto ou número, não vazio e finito.
fn numero(v: Option<&Value>) -> Result<f64> {
    let ok = match v {
        Some(Value::String(s)) => !s.trim().is_empty(),
        Some(Value::Number(_)) => true,
        _ => false,
    };
    let n = js::number(v);
    if ok && n.is_finite() { Ok(n) } else { Err(invalida()) }
}

/// `units()`: inteiro em texto dividido por `10 ** expoente`.
fn unidades(v: Option<&Value>, expoente: i32) -> Result<f64> {
    let Some(Value::String(s)) = v else { return Err(invalida()) };
    let digitos = s.strip_prefix('-').unwrap_or(s);
    if digitos.is_empty() || !digitos.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalida());
    }
    let divisor: f64 = format!("1e{expoente}").parse()?;
    Ok(s.parse::<f64>()? / divisor)
}

/// `isoDate()`.
fn iso_date(v: Option<&Value>) -> String {
    audit::timestamp_ms(v).and_then(js::iso).unwrap_or_default()
}

/// `String(v || '')`.
fn texto(v: Option<&Value>) -> String {
    if js::truthy(v) { js::string(v) } else { String::new() }
}

fn n_json(n: f64) -> Value {
    js::numero_json(n)
}

fn n_texto(n: f64) -> String {
    js::numero_texto(n).unwrap_or_else(|| "NaN".into())
}

/// Cópia com os números no formato do `JSON.stringify`.
fn normalizar(v: &Value) -> Value {
    match v {
        Value::Number(n) => n_json(n.as_f64().unwrap_or(f64::NAN)),
        Value::Array(a) => Value::Array(a.iter().map(normalizar).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), normalizar(v))).collect()),
        outro => outro.clone(),
    }
}

/// `a === b` entre valores simples (objetos nunca são o mesmo).
fn mesmo_valor(a: Option<&Value>, b: Option<&Value>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(Value::Null), Some(Value::Null)) => true,
        (Some(Value::Number(x)), Some(Value::Number(y))) => x.as_f64() == y.as_f64(),
        (Some(Value::String(x)), Some(Value::String(y))) => x == y,
        (Some(Value::Bool(x)), Some(Value::Bool(y))) => x == y,
        _ => false,
    }
}

fn inteiro(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_number).and_then(|n| n.as_f64()).filter(|n| n.is_finite() && n.fract() == 0.0)
}

fn inteiro_seguro(v: Option<&Value>) -> Option<f64> {
    inteiro(v).filter(|n| n.abs() <= 9_007_199_254_740_991.0)
}

/// `/^0x[0-9a-f]{40}$/i`.
fn endereco(v: Option<&Value>) -> Option<String> {
    let s = v?.as_str()?;
    let valido = s.len() == 42 && (s.starts_with("0x") || s.starts_with("0X")) && s[2..].bytes().all(|b| b.is_ascii_hexdigit());
    valido.then(|| s.to_string())
}

/// `collectPages()` com o `page()` de cada cliente.
struct Paginas {
    itens: Vec<Value>,
    vistos: HashSet<String>,
    cursor: String,
}

impl Paginas {
    fn new() -> Self {
        Self { itens: Vec::new(), vistos: HashSet::new(), cursor: String::new() }
    }

    /// Recebe uma página; `true` quando acabou.
    fn receber(&mut self, itens: Option<Value>, cursor: Option<Value>) -> Result<bool> {
        if cursor.as_ref().is_some_and(|c| !c.is_null() && !c.is_string()) {
            return Err(invalida());
        }
        let Some(Value::Array(lista)) = itens else { bail!("Resposta de histórico inválida") };
        self.itens.extend(lista);
        self.cursor = cursor.as_ref().and_then(Value::as_str).unwrap_or("").to_string();
        if self.cursor.is_empty() {
            return Ok(true);
        }
        if !self.vistos.insert(self.cursor.clone()) {
            bail!("Histórico incompleto: cursor repetido");
        }
        Ok(false)
    }
}

const LIMITE_PAGINAS: usize = 1000;

fn tirar(v: &mut Value, chave: &str) -> Option<Value> {
    v.as_object_mut().and_then(|o| o.remove(chave))
}

// ---------------------------------------------------------------------------
// SX Bet
// ---------------------------------------------------------------------------

struct Sx {
    chave: String,
}

impl Sx {
    /// `sxClient()`: exige `status: 'success'` e `data`.
    async fn api(&self, caminho: &str, consulta: &Consulta<'_>, autenticado: bool) -> Result<Value> {
        let cabecalho = autenticado.then_some(("x-sx-api-key", self.chave.as_str()));
        let mut r = pedir(SX, caminho, consulta, cabecalho, PRAZO_PADRAO).await?;
        if r.get("status").and_then(Value::as_str) != Some("success") || r.get("data").is_none_or(Value::is_null) {
            return Err(invalida());
        }
        tirar(&mut r, "data").ok_or_else(invalida)
    }

    async fn posicoes(&self, status: &str) -> Result<Vec<Value>> {
        let mut p = Paginas::new();
        for _ in 0..LIMITE_PAGINAS {
            let mut q: Consulta = vec![("status", status.into()), ("perPage", "100".into())];
            if !p.cursor.is_empty() {
                q.push(("nextKey", p.cursor.clone()));
            }
            let mut d = self.api("/positions-v3", &q, true).await?;
            if p.receber(tirar(&mut d, "positions"), tirar(&mut d, "nextKey"))? {
                return Ok(p.itens);
            }
        }
        bail!("Histórico incompleto: limite de paginação atingido")
    }
}

async fn validar_sx(chave: &str) -> Result<String> {
    let sx = Sx { chave: chave.to_string() };
    let (nada, uma): (Consulta, Consulta) = (vec![], vec![("status", "SETTLED".into()), ("perPage", "1".into())]);
    let (conta, posicoes) = futures::try_join!(sx.api("/user/proxy", &nada, true), sx.api("/positions-v3", &uma, true))?;
    let carteira = endereco(conta.get("obv3ProxyWalletAddress"));
    match carteira {
        Some(c) if posicoes.get("positions").is_some_and(Value::is_array) => Ok(c.to_lowercase()),
        _ => Err(invalida()),
    }
}

/// `readSxBetBalance()`.
async fn saldo_sx(chave: &str) -> Result<(f64, f64)> {
    let sx = Sx { chave: chave.to_string() };
    let nada: Consulta = vec![];
    let (saldos, meta) = futures::try_join!(sx.api("/user/balance-v3", &nada, true), sx.api("/metadata/obv3", &nada, false))?;
    let ativo = meta.get("activeAsset");
    let campo = |k: &str| ativo.and_then(|a| a.get(k));
    let linhas = saldos.get("balances").and_then(Value::as_array);
    let base = endereco(campo("baseToken"));
    let custodia = endereco(campo("escrowAddress"));
    let decimais_ok = campo("decimals").and_then(Value::as_f64) == Some(6.0) && campo("decimals").is_some_and(Value::is_number);
    let (Some(base), Some(custodia), Some(linhas)) = (base, custodia, linhas) else { return Err(invalida()) };
    if campo("symbol").and_then(Value::as_str) != Some("USDC")
        || !decimais_ok
        || linhas.iter().any(|l| endereco(l.get("tokenAddress")).is_none() || endereco(l.get("escrowAddress")).is_none())
    {
        return Err(invalida());
    }
    let (base, custodia) = (base.to_lowercase(), custodia.to_lowercase());
    let iguais: Vec<&Value> = linhas
        .iter()
        .filter(|l| {
            js::string(l.get("tokenAddress")).to_lowercase() == base && js::string(l.get("escrowAddress")).to_lowercase() == custodia
        })
        .collect();
    match iguais.as_slice() {
        [] => Ok((0.0, 0.0)),
        [l] => {
            let saldo = unidades(l.get("availableAmount"), 6)?;
            let travado = unidades(l.get("escrowedAmount"), 6)?;
            if saldo < 0.0 || travado < 0.0 {
                return Err(invalida());
            }
            Ok((saldo, travado))
        }
        _ => Err(invalida()),
    }
}

fn categoria_sx(rotulo: &str) -> &'static str {
    match rotulo {
        "soccer" => "Futebol",
        "basketball" => "Basquete",
        "baseball" => "Beisebol",
        "tennis" => "Tênis",
        "football" | "american football" => "Futebol Americano",
        "hockey" | "ice hockey" => "Hóquei",
        "mma" | "boxing" => "MMA & Boxe",
        "esports" => "Esports",
        "volleyball" => "Vôlei",
        "cricket" => "Críquete",
        "golf" => "Golfe",
        "rugby" => "Rugby",
        _ => "Outros Esportes",
    }
}

/// `sxRow()`.
fn linha_sx(posicao: &Value, mercado: Option<&Value>) -> Result<Map<String, Value>> {
    let hash = match posicao.get("marketHash") {
        Some(Value::String(h)) if !h.is_empty() => h,
        _ => return Err(invalida()),
    };
    let Some(mercado) = mercado.filter(|m| m.get("marketHash").and_then(Value::as_str) == Some(hash.as_str())) else {
        return Err(invalida());
    };
    let stake = unidades(posicao.get("totalStake"), 6)?;
    if stake < 0.0 {
        return Err(invalida());
    }
    let odds = posicao.get("odds");
    let primeiro = odds.and_then(|o| o.get("outcomeOne"));
    let segundo = odds.and_then(|o| o.get("outcomeTwo"));
    let tem_primeiro = js::number(primeiro) > 0.0;
    let tem_segundo = js::number(segundo) > 0.0;
    let resultado: Option<Value> = if tem_primeiro && tem_segundo {
        Some(json!("Ambos os lados"))
    } else if tem_primeiro {
        mercado.get("outcomeOneName").map(normalizar)
    } else if tem_segundo {
        mercado.get("outcomeTwoName").map(normalizar)
    } else {
        Some(json!("Posição"))
    };
    let probabilidade = if tem_primeiro != tem_segundo { Some(unidades(if tem_primeiro { primeiro } else { segundo }, 20)?) } else { None };
    let confronto: Vec<String> =
        [mercado.get("teamOneName"), mercado.get("teamTwoName")].into_iter().filter(|v| js::truthy(*v)).map(js::string).collect();
    let confronto = confronto.join(" vs ");
    let mut partes = Vec::new();
    if !confronto.is_empty() {
        partes.push(confronto);
    }
    if js::truthy(resultado.as_ref()) {
        partes.push(js::string(resultado.as_ref()));
    }
    let titulo = if partes.is_empty() { "Mercado SX Bet".to_string() } else { partes.join(" - ") };
    let rotulo = texto(mercado.get("sportLabel")).to_lowercase();
    let inicio = iso_date(mercado.get("gameTime"));
    let ts = audit::timestamp_ms(posicao.get("betTime")).map(|ms| ms / 1000.0).filter(|v| *v != 0.0 && !v.is_nan());

    let mut m = Map::new();
    m.insert("_platform".into(), json!("sxbet"));
    m.insert("id".into(), json!(hash));
    m.insert("conditionId".into(), json!(hash));
    m.insert("title".into(), json!(titulo));
    if let Some(r) = resultado {
        m.insert("outcome".into(), r);
    }
    m.insert("_category".into(), json!(categoria_sx(rotulo.trim())));
    m.insert("_startDate".into(), if inicio.is_empty() { Value::Null } else { json!(inicio) });
    m.insert("timestamp".into(), ts.map_or(Value::Null, n_json));
    m.insert("initialValue".into(), n_json(stake));
    m.insert("avgPrice".into(), probabilidade.map_or(Value::Null, n_json));
    Ok(m)
}

/// Chave de `Map`/`Set` do JS para um valor simples.
fn chave_valor(v: Option<&Value>, i: usize) -> String {
    match v {
        None => "u".into(),
        Some(Value::Null) => "n".into(),
        Some(Value::String(s)) => format!("s{s}"),
        Some(Value::Number(n)) => format!("d{}", n_texto(n.as_f64().unwrap_or(f64::NAN))),
        Some(Value::Bool(b)) => format!("b{b}"),
        // Objetos só são iguais a si mesmos.
        Some(_) => format!("o{i}"),
    }
}

/// `[...new Map(rows.map(r => [r.marketHash, r])).values()]`: a posição é a
/// da primeira ocorrência e o valor é o da última.
fn unicas(linhas: Vec<Value>) -> Result<Vec<Value>> {
    let mut mapa: IndexMap<String, Value> = IndexMap::new();
    for (i, l) in linhas.into_iter().enumerate() {
        if l.is_null() {
            return Err(invalida());
        }
        mapa.insert(chave_valor(l.get("marketHash"), i), l);
    }
    Ok(mapa.into_values().collect())
}

/// `sxSnapshot()`.
async fn foto_sx(chave: &str) -> Result<Foto> {
    let sx = Sx { chave: chave.to_string() };
    let (abertas, liquidadas) = futures::try_join!(sx.posicoes("LOCKED"), sx.posicoes("SETTLED"))?;
    let mut hashes: IndexMap<String, String> = IndexMap::new();
    for (i, l) in liquidadas.iter().enumerate() {
        if l.is_null() {
            return Err(invalida());
        }
        if audit::timestamp_ms(l.get("market").and_then(|m| m.get("reportedDate"))).is_none() {
            let h = l.get("marketHash");
            let txt = match h {
                None | Some(Value::Null) => String::new(),
                outro => js::string(outro),
            };
            hashes.entry(chave_valor(h, i)).or_insert(txt);
        }
    }
    let hashes: Vec<String> = hashes.into_values().collect();
    let mut mercados: HashMap<String, Value> = HashMap::new();
    for lote in hashes.chunks(30) {
        let d = sx.api("/markets/find", &vec![("marketHashes", lote.join(","))], false).await?;
        let Value::Array(lista) = d else { return Err(invalida()) };
        for m in lista {
            if m.is_null() {
                return Err(invalida());
            }
            if let Some(h) = m.get("marketHash").and_then(Value::as_str) {
                mercados.insert(h.to_string(), m);
            }
        }
    }
    let mut fechadas = Vec::new();
    for p in unicas(liquidadas)? {
        let do_mapa = p.get("marketHash").and_then(Value::as_str).and_then(|h| mercados.get(h));
        let mercado = do_mapa.or(p.get("market").filter(|m| js::truthy(Some(m))));
        let mut l = linha_sx(&p, mercado)?;
        l.insert("realizedPnl".into(), n_json(unidades(p.get("pnl"), 6)?));
        let quando = audit::timestamp_ms(mercado.and_then(|m| m.get("reportedDate")));
        l.insert("_realizedAt".into(), quando.map_or(Value::Null, n_json));
        fechadas.push(Value::Object(l));
    }
    let mut abertas_saida = Vec::new();
    for p in unicas(abertas)? {
        let mut l = linha_sx(&p, p.get("market"))?;
        let lucro = if js::truthy(l.get("avgPrice")) && p.get("netMaxWin").is_some_and(|v| !v.is_null()) {
            n_json(unidades(p.get("netMaxWin"), 6)?)
        } else {
            Value::Null
        };
        l.insert("_potentialProfit".into(), lucro);
        abertas_saida.push(Value::Object(l));
    }
    let sem_data = fechadas.iter().any(|l| !js::truthy(l.get("_realizedAt")));
    Ok(Foto {
        abertas: abertas_saida,
        fechadas,
        aviso: if sem_data { "A SX Bet retornou posições liquidadas sem data oficial.".into() } else { String::new() },
    })
}

// ---------------------------------------------------------------------------
// Predict.fun: categoria e seleção
// ---------------------------------------------------------------------------

regex_js!(PS_VARIANTE, r"^(?:SPORTS_|ESPORTS_)");
regex_js!(PS_LIGA_SLUG, r"^(nfl|nba|ucl)-.+-\d{4}-\d{2}-\d{2}$");
regex_js!(PS_TAG_ESPORTE, r"(?i)^sports?$");

/// `isPredictSports()`.
fn e_esporte(m: Option<&Value>) -> bool {
    let Some(m) = m else { return false };
    js::truthy(m.get("variantDetails").and_then(|v| v.get("sports")))
        || PS_VARIANTE.is_match(&texto(m.get("marketVariant")))
        || m.get("marketType").and_then(Value::as_str).is_some_and(|t| t.starts_with("SPORTS_"))
}

fn rotulo(v: Option<&Value>) -> Option<&'static str> {
    let chave = texto(v).trim().to_lowercase();
    Some(match chave.as_str() {
        "soccer" | "futebol" | "ucl" | "uefa champions league" => "Futebol",
        "basketball" | "basquete" | "nba" | "wnba" | "ncaab" | "euroleague" => "Basquete",
        "american football" | "futebol americano" | "nfl" | "ncaaf" | "cfl" => "Futebol Americano",
        "baseball" | "beisebol" | "mlb" | "npb" | "kbo" => "Beisebol",
        "tennis" | "tênis" | "atp" | "wta" | "itf" => "Tênis",
        "hockey" | "ice hockey" | "hóquei" | "nhl" => "Hóquei",
        "esports" | "cs2" | "dota2" | "league of legends" | "counter-strike 2" | "valorant" => "Esports",
        "volleyball" | "vôlei" | "volei" => "Vôlei",
        "rugby" => "Rugby",
        "cricket" | "críquete" => "Críquete",
        "golf" | "golfe" => "Golfe",
        "motorsport" | "motorsports" | "formula 1" | "f1" | "nascar" | "motogp" => "Automobilismo",
        "mma" | "boxing" | "boxe" | "ufc" => "MMA & Boxe",
        "handball" | "badminton" | "table tennis" | "snooker" | "darts" | "cycling" | "swimming" | "athletics" | "australian rules" => {
            "Outros Esportes"
        }
        _ => return None,
    })
}

fn tags(f: Option<&Value>) -> &[Value] {
    f.and_then(|f| f.get("tags")).and_then(Value::as_array).map_or(&[], |a| a.as_slice())
}

/// `predictCategory(...sources)`.
fn predict_categoria(fontes: &[Option<&Value>]) -> &'static str {
    let mut cats: IndexSet<&'static str> = IndexSet::new();
    let mut ligas: IndexSet<&'static str> = IndexSet::new();
    for &f in fontes {
        let variante = texto(f.and_then(|f| f.get("marketVariant")));
        if variante == "SPORTS_NBA" {
            cats.insert("Basquete");
        }
        if variante.starts_with("SPORTS_FIFA_") {
            cats.insert("Futebol");
        }
        if variante.starts_with("ESPORTS_") {
            cats.insert("Esports");
        }
        for t in tags(f) {
            if let Some(c) = rotulo(t.get("name")) {
                cats.insert(c);
            }
        }
        if e_esporte(f) {
            let f = f.unwrap_or(&Value::Null);
            let slug = texto(titles::ou(f.get("categorySlug"), f.get("slug")));
            let liga = PS_LIGA_SLUG.captures(&slug).map(|c| json!(&c[1]));
            for r in [f.get("team").and_then(|t| t.get("league")), liga.as_ref()] {
                if let Some(c) = rotulo(r) {
                    ligas.insert(c);
                }
            }
        }
    }
    if cats.len() == 1 {
        return cats[0];
    }
    if cats.is_empty() && ligas.len() == 1 {
        return ligas[0];
    }
    let esportiva = fontes.iter().any(|&f| e_esporte(f) || tags(f).iter().any(|t| PS_TAG_ESPORTE.is_match(&texto(t.get("name")))));
    if cats.len() > 1 || esportiva { "Outros Esportes" } else { "Outros" }
}

regex_js!(PP_INTERVALO, r"\bHALFTIME RESULT\b");
regex_js!(PP_CODIGO, r"\bP([1-3])\b");
regex_js!(PP_NOMEADO, r"(?i)\b(first|second|third|fourth|fifth)\s+(quarter|period|set|map)\b");
regex_js!(PP_NUMERADO, r"(?i)\b(\d+)(?:st|nd|rd|th)?\s+(period|set|map)\b|\b(period|set|map)\s+(\d+)\b");

/// `predictPeriod()`.
fn predict_periodo(m: &Value) -> String {
    let tipo = texto(m.get("marketType"));
    let txt = tipo.replace('_', " ");
    if PP_INTERVALO.is_match(&txt) {
        return "1st Half".into();
    }
    let pergunta = texto(m.get("question"));
    let comum = titles::period(&format!("{tipo} {pergunta}"));
    if !comum.is_empty() {
        return comum;
    }
    let codigo = PP_CODIGO.captures(&txt).map_or(String::new(), |c| format!("Period {} ", &c[1]));
    let fonte = format!("{codigo}{txt} {pergunta}");
    let nomeado = PP_NOMEADO.captures(&fonte);
    let numerado = PP_NUMERADO.captures(&fonte);
    let n: f64 = match &nomeado {
        Some(c) => ["first", "second", "third", "fourth", "fifth"].iter().position(|w| *w == c[1].to_lowercase()).map_or(0.0, |i| i as f64 + 1.0),
        None => numerado.as_ref().and_then(|c| c.get(1).or(c.get(4))).map_or(f64::NAN, |g| g.as_str().parse().unwrap_or(f64::NAN)),
    };
    if n == 0.0 || n.is_nan() {
        let escopo = m.get("_predictScope");
        return if js::truthy(escopo) { titles::period(&js::string(escopo)) } else { String::new() };
    }
    let unidade = nomeado
        .as_ref()
        .and_then(|c| c.get(2))
        .or_else(|| numerado.as_ref().and_then(|c| c.get(2).or(c.get(3))))
        .map_or(String::new(), |g| g.as_str().to_lowercase());
    format!("{} {}", titles::ordinal(n as i64), titles::capitalizar(&unidade))
}

fn junta(partes: &[&str]) -> String {
    partes.iter().filter(|p| !p.is_empty()).copied().collect::<Vec<_>>().join(" ")
}

regex_js!(SIM_NAO, r"(?i)^(?:yes|no)$");
regex_js!(SO_NAO, r"(?i)^No$");
regex_js!(PS_QUALIF, r"(?i)\((?:excluding overtime|including overtime|regulation only)\)");
regex_js!(PS_QUALIF_ESPACO, r"(?i)\s*\((?:excluding overtime|including overtime|regulation only)\)\s*");
regex_js!(PS_ANTES_PONTUACAO, r"\s+([?:])");
regex_js!(PS_NO_FINAL, r"(?i)\sNo$");
regex_js!(
    PS_INF_SPREAD,
    r"(?i)^(?:(?:1st|2nd) Half\s+)?(?:Spread|Handicap):\s*.+?\s*\([+-]\d+(?:\.\d+)?\)(?:\s+vs\.?\s+.+?\s*\([+-]\d+(?:\.\d+)?\))?$"
);
regex_js!(PS_INF_AMBAS, r"(?i)^Both teams to score\??$");
regex_js!(
    PS_INF_TOTAL,
    r"(?i)^(?:Over|Under|O/U|Over/Under|Total(?: Goals| Points| Runs| Corners| Games| Sets)?)\s+(?:Over\s+|Under\s+)?\d+(?:\.\d+)?(?:\s+(?:goals|points|runs|corners|games|sets))?\??$"
);
regex_js!(PS_VENCE, r"(?i)^Will (.+?) win(?: on \d{4}-\d{2}-\d{2})?\?$");
regex_js!(
    PS_CONDICIONAL,
    r"(?i)\b(?:if|unless|provided that|combined with)\b|\b(?:and|or)\s+(?:will\s+)?(?:win|wins|score|scores|lose|finish|reach|record|cover|be|have|get|take|over|under)\b|\b(?:wins?|scores?|reaches?|records?|covers?)\b.*\b(?:and|or)\b"
);
regex_js!(PS_LINHA_EXPLICITA, r"(?i)\b(?:Over|Under|O/U|Over/Under)\s+(\d+(?:\.\d+)?)");
regex_js!(PS_CORRIDA, r"\bRace to (\d+) pontos");
regex_js!(PS_TIPO_VENCEDOR, r"(?:MONEYLINE|_WINNER|HALFTIME_RESULT|SECOND_HALF_RESULT)$");
regex_js!(PS_AVANCA, r"(?i)^(?:Will\s+)?(.+?)\s+(?:advance|qualify)(?:\s+to\s+(?:the\s+)?next\s+round)?\??$");
regex_js!(
    PS_TIPO_ML,
    r"^SPORTS_(?:(?:(?:FIRST|SECOND)_HALF|Q[1-4]|(?:FIRST|SECOND|THIRD|FOURTH)_QUARTER)_)?(?:MONEYLINE|CHILD_MONEYLINE|HALFTIME_RESULT|SECOND_HALF_RESULT)$"
);
regex_js!(PS_EMPATE, r"(?i)^(?:draw|tie)$");

fn linha_variante(m: &Value) -> Option<&Value> {
    m.get("variantDetails").and_then(|v| v.get("sports")).and_then(|s| s.get("line"))
}

fn definido(v: Option<&Value>) -> Option<&Value> {
    v.filter(|v| !v.is_null())
}

fn num_texto(s: &str) -> f64 {
    js::number(Some(&Value::String(s.to_string())))
}

fn nome_resultado(o: &Value) -> String {
    js::string(o.get("name")).trim().to_string()
}

/// `predictSelection()`.
fn predict_selecao(m: &Value, o: &Value, categoria: &str) -> String {
    let pergunta = m.get("question").and_then(Value::as_str).map_or(String::new(), |s| s.trim().to_string());
    let lado = nome_resultado(o);
    let qualificadores: Vec<&str> = PS_QUALIF.find_iter(&pergunta).map(|q| q.as_str()).collect();
    if !qualificadores.is_empty() {
        let limpa = PS_QUALIF_ESPACO.replace_all(&pergunta, " ");
        let limpa = PS_ANTES_PONTUACAO.replace_all(&limpa, "$1").trim().to_string();
        let mut sem = m.clone();
        if let Some(obj) = sem.as_object_mut() {
            obj.insert("question".into(), json!(limpa));
        }
        let detalhe = predict_selecao(&sem, o, categoria);
        let mut unicos: Vec<&str> = Vec::new();
        for q in qualificadores {
            if !unicos.contains(&q) {
                unicos.push(q);
            }
        }
        let qualificador = unicos.join(" ");
        return if PS_NO_FINAL.is_match(&detalhe) {
            PS_NO_FINAL.replace(&detalhe, regex::NoExpand(&format!(" {qualificador} No"))).into_owned()
        } else {
            format!("{detalhe} {qualificador}")
        };
    }
    // Registros antigos não têm marketType: só uma família completa e explícita é inferida.
    let mut inferido_m: Option<Value> = None;
    if !js::truthy(m.get("marketType")) && (e_esporte(Some(m)) || categoria != "Outros") {
        let inferido = if PS_INF_SPREAD.is_match(&pergunta) {
            "SPORTS_SPREADS"
        } else if PS_INF_AMBAS.is_match(&pergunta) {
            "SPORTS_BOTH_TEAMS_TO_SCORE"
        } else if PS_INF_TOTAL.is_match(&pergunta) {
            "SPORTS_TOTALS"
        } else if PS_VENCE.is_match(&pergunta) {
            "SPORTS_MONEYLINE"
        } else {
            ""
        };
        if !inferido.is_empty() {
            let mut c = m.clone();
            if let Some(obj) = c.as_object_mut() {
                obj.insert("marketType".into(), json!(inferido));
            }
            inferido_m = Some(c);
        }
    }
    let m = inferido_m.as_ref().unwrap_or(m);
    let periodo = predict_periodo(m);
    let escopado = |s: &str| junta(&[&periodo, s]);
    let esporte = e_esporte(Some(m)) || categoria != "Outros";
    let tipo = texto(m.get("marketType"));
    let nao = SO_NAO.is_match(&lado);
    if PS_CONDICIONAL.is_match(&pergunta) {
        let l = if lado.eq_ignore_ascii_case("yes") {
            "Sim".to_string()
        } else if nao {
            "Não".to_string()
        } else {
            lado.clone()
        };
        return format!("{pergunta} — {l}");
    }
    let linha = linha_variante(m);
    if esporte {
        if let (Some(c), Some(l)) = (PS_LINHA_EXPLICITA.captures(&pergunta), definido(linha)) {
            if num_texto(&c[1]) != js::number(Some(l)) {
                return format!("{pergunta} — {lado}");
            }
        }
    }
    if esporte && !pergunta.is_empty() {
        let detalhe = titles::selection_detail(&pergunta, &lado, &periodo);
        let corrida = PS_CORRIDA.captures(&detalhe);
        let linha_ok = match (&corrida, definido(linha)) {
            (None, _) | (_, None) => true,
            (Some(c), Some(l)) => num_texto(&c[1]) == js::number(Some(l)),
        };
        if !detalhe.is_empty() && linha_ok {
            return detalhe;
        }
        // Reaproveita os formatadores exatos; nunca empresta um mercado irmão.
        let tipo_esporte = match tipo.as_str() {
            "SPORTS_CHILD_MONEYLINE" => "child_moneyline".to_string(),
            "SPORTS_PLAYER_ASSISTS" => "assists".to_string(),
            "SPORTS_PLAYER_SHOTS_ON_TARGET" => "soccer_player_shots_on_target".to_string(),
            "SPORTS_BOTH_TEAMS_TO_SCORE_FIRST_HALF" => "both_teams_to_score_first_half".to_string(),
            "SPORTS_BOTH_TEAMS_TO_SCORE_SECOND_HALF" => "both_teams_to_score_second_half".to_string(),
            t => t.strip_prefix("SPORTS_").unwrap_or(t).to_lowercase(),
        };
        let formatado = titles::extended_title_exato(&pergunta, &tipo_esporte, &lado, linha);
        if !formatado.is_empty() && !formatado.contains("Mercado não confirmado:") {
            return formatado;
        }
    }
    if esporte {
        for f in [predict_handicap(m, o, &periodo), predict_total(m, o, categoria, &periodo), predict_especial(m, o, categoria, &periodo)] {
            if !f.is_empty() {
                return f;
            }
        }
    }
    if SIM_NAO.is_match(&lado) {
        if pergunta.is_empty() {
            return lado;
        }
        let sufixo = if nao { " No" } else { "" };
        if esporte {
            if let Some(c) = PS_VENCE.captures(&pergunta) {
                if tipo.is_empty() || PS_TIPO_VENCEDOR.is_match(&tipo) {
                    return escopado(&format!("{} ML{sufixo}", &c[1]));
                }
            }
        }
        let total = titles::compact_kalshi_total(&pergunta, &lado, &periodo);
        if esporte && !total.is_empty() {
            return total;
        }
        if esporte && tipo == "SPORTS_TEAM_TO_ADVANCE" {
            if let Some(c) = PS_AVANCA.captures(&pergunta) {
                return escopado(&format!("{} Advances{sufixo}", &c[1]));
            }
        }
        return format!("{pergunta} — {}", if nao { "Não" } else { "Sim" });
    }
    if esporte && PS_TIPO_ML.is_match(&tipo) {
        let prefixo = if tipo == "SPORTS_HALFTIME_RESULT" { "1st Half".to_string() } else { periodo.clone() };
        let selecao = if PS_EMPATE.is_match(&lado) { "Empate".to_string() } else { format!("{lado} ML") };
        return junta(&[&prefixo, &selecao]);
    }
    if esporte && tipo == "SPORTS_TEAM_TO_ADVANCE" {
        return escopado(&format!("{lado} Advances"));
    }
    if lado == pergunta || pergunta.is_empty() { lado } else { format!("{lado} · {pergunta}") }
}

regex_js!(SE_NENHUM, r"(?i)^(?:Neither(?: team)?|None|No team)$");
regex_js!(SE_QUALQUER, r"(?i)^(?:Will\s+)?(?:any|either) team\s+(?:reach|score)\s+(\d+)\s+points?(?:\s+first)?\??$");
regex_js!(SE_PLACAR_PERGUNTA, r"(?i)^(?:Will\s+)?(?:the\s+)?(?:final\s+)?score\s+(?:be\s+)?(\d+\s*[-:]\s*\d+)\??$");
regex_js!(SE_PLACAR_LADO, r"^\d+\s*[-:]\s*\d+$");
regex_js!(SE_PLACAR_SEP, r"\s*[-:]\s*");
regex_js!(SE_PRIMEIRO_GOL, r"(?i)^(?:Will\s+)?(.+?)\s+(?:score\s+(?:the\s+)?first\s+goal|(?:be\s+the\s+)?first\s+(?:team\s+)?to\s+score)\??$");
regex_js!(SE_GOL, r"(?i)\bgoal\b");
regex_js!(SE_ESCANTEIO, r"(?i)^(?:Will\s+)?(.+?)\s+(?:get|take|win)\s+(?:the\s+)?first\s+corner\??$");
regex_js!(SE_FIRST_BLOOD, r"(?i)^(?:Will\s+)?(.+?)\s+(?:get|take|draw|score)\s+(?:the\s+)?first\s+blood\??$");
regex_js!(
    SE_TD,
    r"(?i)^(?:Will\s+)?(.+?)\s+(?:score\s+(?:a|an|anytime)\s+(?:TD|touchdown)(?:\s+anytime)?|score\s+(?:a\s+)?(?:TD|touchdown)\s+at\s+any\s+time)\??$"
);
regex_js!(
    SE_PAR_PERGUNTA,
    r"(?i)^(?:Will\s+)?(?:the\s+)?(?:total(?:\s+(?:goals|points|runs|corners))?|number of corners)\s+(?:be\s+)?(odd|even)\??$"
);
regex_js!(SE_PAR_LADO, r"(?i)^(odd|even)$");
regex_js!(SE_IMPAR, r"(?i)^odd$");
regex_js!(SE_PRORROGACAO, r"(?i)^(?:Will\s+)?(?:the\s+)?(?:game|match)\s+(?:go\s+to|have)\s+extra\s+time\??$");
regex_js!(SE_PENALTIS, r"(?i)^(?:Will\s+)?(?:the\s+)?(?:game|match)\s+(?:go\s+to|have)\s+(?:a\s+)?penalty\s+shootout\??$");

/// `predictSpecial()`.
fn predict_especial(m: &Value, o: &Value, categoria: &str, periodo: &str) -> String {
    let tipo = texto(m.get("marketType"));
    let pergunta = texto(m.get("question")).trim().to_string();
    let lado = nome_resultado(o);
    let binario = SIM_NAO.is_match(&lado);
    let negativo = if SO_NAO.is_match(&lado) { " No" } else { "" };
    let escopo = |t: &str| junta(&[periodo, t]);
    let participante = |re: &regex::Regex| -> String {
        if !binario {
            return if SE_NENHUM.is_match(&lado) { "Nenhuma equipe".into() } else { lado.clone() };
        }
        re.captures(&pergunta).and_then(|c| c.get(1)).map_or(String::new(), |g| g.as_str().trim().to_string())
    };
    if tipo == "SPORTS_RACE_TO_POINTS" {
        if let Some(c) = SE_QUALQUER.captures(&pergunta) {
            let n = num_texto(&c[1]);
            if binario && n > 0.0 {
                return escopo(&format!("Qualquer equipe Race to {} pontos{negativo}", n_texto(n)));
            }
        }
    }
    if tipo == "SPORTS_EXACT_SCORE" {
        let placar = if binario {
            SE_PLACAR_PERGUNTA.captures(&pergunta).and_then(|c| c.get(1)).map(|g| g.as_str().to_string())
        } else {
            SE_PLACAR_LADO.find(&lado).map(|g| g.as_str().to_string())
        };
        if let Some(p) = placar.filter(|p| !p.is_empty()) {
            return escopo(&format!("Placar exato {}{negativo}", SE_PLACAR_SEP.replace(&p, "-")));
        }
    }
    if tipo == "SPORTS_FIRST_TO_SCORE" {
        let time = participante(&SE_PRIMEIRO_GOL);
        if !time.is_empty() && (categoria == "Futebol" || categoria == "Hóquei" || SE_GOL.is_match(&pergunta)) {
            return escopo(&format!("{time} First Goal{negativo}"));
        }
    }
    let primeiro: Option<(&str, &regex::Regex)> = match tipo.as_str() {
        "SPORTS_FIRST_CORNER" => Some(("First Corner", &SE_ESCANTEIO)),
        "SPORTS_FIRST_BLOOD_GAME" => Some(("First Blood", &SE_FIRST_BLOOD)),
        "SPORTS_ANYTIME_TOUCHDOWNS" => Some(("Anytime TD", &SE_TD)),
        _ => None,
    };
    if let Some((rotulo, re)) = primeiro {
        let time = participante(re);
        if !time.is_empty() {
            return escopo(&format!("{time} {rotulo}{negativo}"));
        }
    }
    if tipo == "SPORTS_ODD_EVEN" || tipo == "SPORTS_CORNERS_ODD_EVEN" {
        let rotulo = if tipo == "SPORTS_CORNERS_ODD_EVEN" { "Escanteios" } else { "Total" };
        let par = if binario {
            SE_PAR_PERGUNTA.captures(&pergunta).map(|c| c[1].to_string())
        } else {
            SE_PAR_LADO.captures(&lado).map(|c| c[1].to_string())
        };
        if let Some(p) = par {
            return escopo(&format!("{rotulo} {}{negativo}", if SE_IMPAR.is_match(&p) { "ímpar" } else { "par" }));
        }
    }
    let extra: Option<(&str, &regex::Regex)> = match tipo.as_str() {
        "SPORTS_EXTRA_TIME" => Some(("Extra Time", &SE_PRORROGACAO)),
        "SPORTS_PENALTY_SHOOTOUT" => Some(("Penalty Shootout", &SE_PENALTIS)),
        _ => None,
    };
    if let Some((rotulo, re)) = extra {
        if binario && re.is_match(&pergunta) {
            return format!("{rotulo} {}", if SO_NAO.is_match(&lado) { "No" } else { "Yes" });
        }
    }
    String::new()
}

regex_js!(
    PH_TIPO,
    r"^SPORTS_(?:(?:FIRST|SECOND)_HALF_|Q[1-4]_)?(?:SPREADS?|MAP_HANDICAP|ROUND_HANDICAP|TENNIS_SET_HANDICAP|TENNIS_GAME_HANDICAP)$"
);
regex_js!(PH_LADO, r"^(.+?)\s+\(?([+-]\d+(?:\.\d+)?)\)?$");
regex_js!(
    PH_SPREAD,
    r"(?i)^(?:(?:1st|2nd) Half\s+)?(?:(?:Map|Game|Set|Round)\s+)?(?:Spread|Handicap):\s*(.+?)\s*\(([+-]\d+(?:\.\d+)?)\)(?:\s+vs\.?\s+(.+?)\s*\(([+-]\d+(?:\.\d+)?)\))?$"
);
regex_js!(PH_SIM_NAO, r"(?i)^(Yes|No)$");

fn norm(v: Option<&Value>) -> String {
    texto(v).trim().to_lowercase()
}

fn nomes(c: Option<&Value>) -> [String; 3] {
    let c = c.unwrap_or(&Value::Null);
    [
        norm(c.get("name")),
        norm(c.get("variantDetails").and_then(|v| v.get("sports")).and_then(|s| s.get("team")).and_then(|t| t.get("name"))),
        norm(c.get("team").and_then(|t| t.get("name"))),
    ]
}

/// `predictHandicap()`.
fn predict_handicap(m: &Value, o: &Value, periodo: &str) -> String {
    let tipo = texto(m.get("marketType"));
    if !PH_TIPO.is_match(&tipo) {
        return String::new();
    }
    let pergunta = texto(m.get("question")).trim().to_string();
    let lado = nome_resultado(o);
    let binario = PH_SIM_NAO.is_match(&lado);
    let prefixo = if tipo.ends_with("MAP_HANDICAP") {
        "Handicap de mapas:"
    } else if tipo.ends_with("TENNIS_SET_HANDICAP") {
        "Handicap de sets:"
    } else if tipo.ends_with("TENNIS_GAME_HANDICAP") {
        "Handicap de games:"
    } else {
        ""
    };
    if !binario {
        if let Some(s) = PH_LADO.captures(&lado) {
            return junta(&[prefixo, &s[1], periodo, &s[2]]);
        }
    }
    let Some(spread) = PH_SPREAD.captures(&pergunta) else { return String::new() };
    let alvo1 = spread[1].trim().to_lowercase();
    if binario {
        return junta(&[prefixo, &spread[1], periodo, &spread[2], if SO_NAO.is_match(&lado) { "No" } else { "" }]);
    }
    let meus = nomes(Some(o));
    let linha: Option<f64> = if meus.contains(&alvo1) {
        Some(num_texto(&spread[2]))
    } else if let (Some(t3), Some(l4)) = (spread.get(3), spread.get(4)) {
        meus.contains(&t3.as_str().trim().to_lowercase()).then(|| num_texto(l4.as_str()))
    } else {
        let resultados: &[Value] = m.get("outcomes").and_then(Value::as_array).map_or(&[], |a| a.as_slice());
        let com_alvo = resultados.iter().filter(|c| nomes(Some(c)).contains(&alvo1)).count();
        let mesmo = resultados.iter().any(|c| mesmo_valor(c.get("indexSet"), o.get("indexSet")));
        (resultados.len() == 2 && com_alvo == 1 && mesmo).then(|| -num_texto(&spread[2]))
    };
    let Some(n) = linha else { return String::new() };
    let numero = format!("{}{}", if n > 0.0 { "+" } else { "" }, n_texto(n));
    junta(&[prefixo, &lado, periodo, &numero])
}

regex_js!(
    PT_ORDINARIO,
    r"^SPORTS_(?:(?:FIRST|SECOND)_HALF_|Q[1-4]_|P[1-3]_)?(?:(?:TEAM_)?TOTALS|(?:TEAM_)?TOTAL_CORNERS)$"
);
regex_js!(PT_UNIDADE, r"(?i)\bTotal\s+(Goals|Points|Runs|Corners|Games|Sets)\b|\b\d+(?:\.\d+)?\s+(goals|points|runs|corners|games|sets)\s*\??$");
regex_js!(PT_LADO, r"(?i)^(Over|Under)(?:\s+(\d+(?:\.\d+)?))?$");
regex_js!(PT_BINARIO, r"(?i)^(Over|Under)\s+(\d+(?:\.\d+)?)(?:\s+(?:goals|points|runs|corners|games|sets|rounds))?\??$");
regex_js!(PT_LINHA, r"(?i)\b(?:O/U|Over/Under|Over|Under|Total(?: Goals| Points| Runs| Corners| Games| Sets)?)\s+(\d+(?:\.\d+)?)(?:\s|$)");
regex_js!(PT_TIME, r"TEAM_(?:TOTALS|TOTAL_CORNERS)$");
regex_js!(
    PT_PROP,
    r"(?i)^([^:]+):\s*(?:Goals(?:\s*\+\s*Assists)?|Assists|Shots(?: on Target)?|Goalkeeper Saves|Saves)\s+(?:O/U|Over/Under|Over|Under)\s+\d+(?:\.\d+)?$"
);

/// `predictTotal()`.
fn predict_total(m: &Value, o: &Value, categoria: &str, periodo: &str) -> String {
    let tipo = texto(m.get("marketType"));
    let lado = nome_resultado(o);
    let unidade_tipo = match tipo.as_str() {
        "SPORTS_PLAYER_GOALS" => Some("gols"),
        "SPORTS_PLAYER_ASSISTS" => Some("assistências"),
        "SPORTS_PLAYER_GOALS_PLUS_ASSISTS" => Some("gols + assistências"),
        "SPORTS_PLAYER_SHOTS" => Some("chutes"),
        "SPORTS_PLAYER_SHOTS_ON_TARGET" => Some("chutes no alvo"),
        "SPORTS_PLAYER_GOALKEEPER_SAVES" => Some("defesas"),
        "SPORTS_ROUND_TOTALS" => Some("rounds"),
        "SPORTS_KILL_OVER_UNDER_GAME" => Some("kills"),
        "SPORTS_TENNIS_SET_TOTALS" => Some("sets"),
        "SPORTS_TENNIS_MATCH_TOTALS" | "SPORTS_TENNIS_FIRST_SET_TOTALS" => Some("games"),
        _ => None,
    };
    if !PT_ORDINARIO.is_match(&tipo) && unidade_tipo.is_none() {
        return String::new();
    }
    let pergunta = texto(m.get("question")).trim().to_string();
    let unidade = match unidade_tipo {
        Some(u) => u,
        None if tipo.ends_with("CORNERS") => "escanteios",
        None => match PT_UNIDADE.captures(&pergunta) {
            Some(c) => match c.get(1).or(c.get(2)).map_or(String::new(), |g| g.as_str().to_lowercase()).as_str() {
                "goals" => "gols",
                "points" => "pontos",
                "runs" => "runs",
                "corners" => "escanteios",
                "games" => "games",
                "sets" => "sets",
                _ => "",
            },
            None => match titles::sport_unit(categoria) {
                "" => match categoria {
                    "Rugby" | "Vôlei" => "pontos",
                    "Críquete" => "runs",
                    _ => "",
                },
                u => u,
            },
        },
    };
    if unidade.is_empty() {
        return String::new();
    }
    let binario = if SIM_NAO.is_match(&lado) { PT_BINARIO.captures(&pergunta) } else { None };
    let (direcao, linha_lado) = match PT_LADO.captures(&lado) {
        Some(c) => (c[1].to_string(), c.get(2).map(|g| g.as_str().to_string())),
        None => match &binario {
            Some(b) => (b[1].to_string(), b.get(2).map(|g| g.as_str().to_string())),
            None => return String::new(),
        },
    };
    let linha_pergunta = PT_LINHA.captures(&pergunta).map(|c| c[1].to_string());
    let linha_var = definido(linha_variante(m));
    // `selected[2] ?? questionLine ?? line`
    let linha = match (&linha_lado, &linha_pergunta, linha_var) {
        (Some(s), _, _) | (None, Some(s), _) => num_texto(s),
        (None, None, Some(v)) => js::number(Some(v)),
        (None, None, None) => return String::new(),
    };
    if !linha.is_finite() || linha < 0.0 {
        return String::new();
    }
    let diverge = linha_lado.iter().chain(linha_pergunta.iter()).any(|s| num_texto(s) != linha)
        || linha_var.is_some_and(|v| js::number(Some(v)) != linha);
    if diverge {
        return String::new();
    }
    let mut sujeito = String::new();
    if PT_TIME.is_match(&tipo) {
        let vd = m.get("variantDetails").and_then(|v| v.get("sports")).and_then(|s| s.get("team")).and_then(|t| t.get("name"));
        sujeito = texto(titles::ou(vd, m.get("team").and_then(|t| t.get("name"))));
        if sujeito.is_empty() {
            return String::new();
        }
    }
    if tipo.starts_with("SPORTS_PLAYER_") {
        let Some(p) = PT_PROP.captures(&pergunta) else { return String::new() };
        sujeito = p[1].to_string();
    }
    let escopo = if tipo == "SPORTS_TENNIS_FIRST_SET_TOTALS" { "1st Set" } else { periodo };
    let mut direcao = titles::capitalizar(&direcao);
    let negativo = binario.is_some() && SO_NAO.is_match(&lado);
    let inverter = negativo && linha.fract() != 0.0 && unidade != "rounds";
    if inverter {
        direcao = if direcao == "Over" { "Under".into() } else { "Over".into() };
    }
    let numero = n_texto(linha);
    junta(&[escopo, &sujeito, &direcao, &numero, unidade, if negativo && !inverter { "No" } else { "" }])
}

// ---------------------------------------------------------------------------
// Predict.fun: posições, preenchimentos e conciliação
// ---------------------------------------------------------------------------

struct Predict {
    chave: String,
}

impl Predict {
    /// `predictClient()`: exige `success: true` e `data`; devolve a resposta inteira.
    async fn api(&self, caminho: &str, consulta: &Consulta<'_>, prazo: Duration) -> Result<Value> {
        let r = pedir(PREDICT, caminho, consulta, Some(("x-api-key", self.chave.as_str())), prazo).await?;
        if r.get("success") != Some(&Value::Bool(true)) || r.get("data").is_none_or(Value::is_null) {
            return Err(invalida());
        }
        Ok(r)
    }

    async fn todas(&self, caminho: &str, consulta: Consulta<'_>) -> Result<Vec<Value>> {
        let mut p = Paginas::new();
        for _ in 0..LIMITE_PAGINAS {
            let mut q = consulta.clone();
            if !p.cursor.is_empty() {
                q.push(("after", p.cursor.clone()));
            }
            let mut r = self.api(caminho, &q, PRAZO_PADRAO).await?;
            if p.receber(tirar(&mut r, "data"), tirar(&mut r, "cursor"))? {
                return Ok(p.itens);
            }
        }
        bail!("Histórico incompleto: limite de paginação atingido")
    }
}

async fn validar_predict(chave: &str, carteira: &str) -> Result<String> {
    let p = Predict { chave: chave.to_string() };
    let r = p.api(&format!("/positions/{carteira}"), &vec![("first", "1".into())], PRAZO_PADRAO).await?;
    if !r.get("data").is_some_and(Value::is_array) {
        return Err(invalida());
    }
    Ok(carteira.to_string())
}

/// `validatePredictIdentity()`.
fn identidade(m: Option<&Value>, o: Option<&Value>) -> Result<(String, f64, f64)> {
    let id = inteiro_seguro(m.and_then(|m| m.get("id")));
    let indice = inteiro(o.and_then(|o| o.get("indexSet")));
    let nome_ok = o.and_then(|o| o.get("name")).and_then(Value::as_str).is_some_and(|n| !n.is_empty());
    let titulo_ok = js::truthy(m.and_then(|m| m.get("title")));
    match (id, indice) {
        (Some(id), Some(i)) if nome_ok && titulo_ok => Ok((format!("{}:{}", n_texto(id), n_texto(i)), id, i)),
        _ => Err(invalida()),
    }
}

/// `predictRow()`.
fn linha_predict(m: &Value, o: &Value, id: f64, indice: f64) -> Map<String, Value> {
    let inicio = if e_esporte(Some(m)) { iso_date(m.get("speedBump").and_then(|s| s.get("liveFrom"))) } else { String::new() };
    let categoria = predict_categoria(&[Some(m)]);
    let truthy_ou_vazio = |v: Option<&Value>| if js::truthy(v) { normalizar(v.unwrap_or(&Value::Null)) } else { json!("") };
    let mut r = Map::new();
    r.insert("_platform".into(), json!("predict"));
    r.insert("conditionId".into(), json!(n_texto(id)));
    r.insert("title".into(), normalizar(m.get("title").unwrap_or(&Value::Null)));
    r.insert("outcome".into(), o.get("name").cloned().unwrap_or(Value::Null));
    r.insert("outcomeIndex".into(), n_json(indice));
    r.insert("_marketSlug".into(), truthy_ou_vazio(m.get("categorySlug")));
    r.insert("_predictMarketType".into(), truthy_ou_vazio(m.get("marketType")));
    r.insert("_predictSelection".into(), json!(predict_selecao(m, o, categoria)));
    if !inicio.is_empty() {
        r.insert("_startDate".into(), json!(inicio));
    }
    r.insert("_category".into(), json!(categoria));
    r
}

struct Preenchimento {
    compra: bool,
    tempo: f64,
    preco: f64,
    cotas: f64,
    caixa: f64,
    id: String,
}

/// `predictFill()`.
fn preenchimento(mt: &Value, perna: &Value, tomador: bool) -> Result<Preenchimento> {
    let tamanho = unidades(if tomador { mt.get("amountFilled") } else { perna.get("amount") }, 18)?;
    let preco = unidades(if tomador { mt.get("priceExecuted") } else { perna.get("price") }, 18)?;
    let taxa_v = perna.get("fee");
    let taxa = unidades(taxa_v.and_then(|f| f.get("amount")), 18)?;
    let tipo_cotacao = perna.get("quoteType").and_then(Value::as_str);
    let compra = tipo_cotacao == Some("Bid");
    let tempo = audit::timestamp_ms(mt.get("executedAt")).filter(|t| *t != 0.0);
    let tipo_taxa = taxa_v.and_then(|f| f.get("type")).and_then(Value::as_str).unwrap_or("");
    let valido = tamanho > 0.0 && preco > 0.0 && preco <= 1.0 && taxa >= 0.0 && tempo.is_some();
    if !valido || (!compra && tipo_cotacao != Some("Ask")) || !["COLLATERAL", "SHARES"].contains(&tipo_taxa) {
        return Err(invalida());
    }
    let taxa_cotas = if tipo_taxa == "SHARES" { taxa } else { 0.0 };
    let taxa_caixa = if tipo_taxa == "COLLATERAL" { taxa } else { 0.0 };
    Ok(Preenchimento {
        compra,
        tempo: tempo.unwrap_or(0.0),
        preco,
        cotas: tamanho + if compra { -taxa_cotas } else { taxa_cotas },
        caixa: tamanho * preco + if compra { taxa_caixa } else { -taxa_caixa },
        id: format!("{}:{}:{}", js::string(mt.get("transactionHash")), texto(mt.get("settlementId")), js::string(perna.get("hash"))),
    })
}

struct Grupo {
    linha: Map<String, Value>,
    mercado: Value,
    preenchimentos: IndexMap<String, Preenchimento>,
    resgates: Vec<Value>,
}

impl Grupo {
    fn resultado_mantido(&self) -> Option<&Value> {
        let indice = self.linha.get("outcomeIndex");
        self.mercado.get("outcomes").and_then(Value::as_array)?.iter().find(|o| mesmo_valor(o.get("indexSet"), indice))
    }

    fn resolucao_binaria(&self) -> bool {
        let v = self.mercado.get("resolution").and_then(|r| r.get("indexSet"));
        v.and_then(Value::as_number).and_then(|n| n.as_f64()).is_some_and(|n| n == 1.0 || n == 2.0)
    }

    fn status_mantido(&self) -> Option<&str> {
        self.resultado_mantido().and_then(|o| o.get("status")).and_then(Value::as_str)
    }
}

struct Estoque {
    cotas: f64,
    custo: f64,
    entrada: f64,
    tempo_entrada: f64,
}

fn vender(g: &Grupo, e: &mut Estoque, fechadas: &mut Vec<Map<String, Value>>, id: &str, tempo: f64, qtd: f64, pagamento: f64) {
    let stake = e.custo * qtd / e.cotas;
    let bruto = e.entrada * qtd / e.cotas;
    let mut r = g.linha.clone();
    r.insert("id".into(), json!(id));
    r.insert("avgPrice".into(), n_json(bruto / qtd));
    r.insert("initialValue".into(), n_json(stake));
    r.insert("size".into(), n_json(qtd));
    r.insert("timestamp".into(), n_json(e.tempo_entrada / 1000.0));
    r.insert("realizedPnl".into(), n_json(pagamento - stake));
    r.insert("_realizedAt".into(), n_json(tempo));
    fechadas.push(r);
    e.cotas -= qtd;
    e.custo -= stake;
    e.entrada -= bruto;
}

/// `reconcilePredict()`. `Ok(None)` = operações que não fecham.
fn conciliar(g: &Grupo, mantida: Option<f64>, resolucao: Option<f64>) -> Result<Option<Vec<Map<String, Value>>>> {
    let mut e = Estoque { cotas: 0.0, custo: 0.0, entrada: 0.0, tempo_entrada: 0.0 };
    let mut fechadas = Vec::new();
    let mut ordem: Vec<&Preenchimento> = g.preenchimentos.values().collect();
    ordem.sort_by(|a, b| a.tempo.partial_cmp(&b.tempo).unwrap_or(std::cmp::Ordering::Equal));
    for p in ordem {
        if p.cotas <= 0.0 || p.caixa < 0.0 {
            return Ok(None);
        }
        if p.compra {
            if e.cotas <= TOLERANCIA {
                e.tempo_entrada = p.tempo;
            }
            e.cotas += p.cotas;
            e.custo += p.caixa;
            e.entrada += p.preco * p.cotas;
        } else {
            if e.cotas <= TOLERANCIA || p.cotas > e.cotas + TOLERANCIA {
                return Ok(None);
            }
            let qtd = p.cotas.min(e.cotas);
            vender(g, &mut e, &mut fechadas, &p.id, p.tempo, qtd, p.caixa);
        }
    }
    let restante = mantida.unwrap_or(0.0);
    let resolucao = resolucao.filter(|r| *r != 0.0);
    if let Some(resgate) = g.resgates.first() {
        let tempo = audit::timestamp_ms(resgate.get("createdAt")).filter(|t| *t != 0.0);
        let status = g.status_mantido();
        let (Some(tempo), Some(res)) = (tempo, resolucao) else { return Ok(None) };
        if g.resgates.len() != 1
            || restante > TOLERANCIA
            || e.cotas <= TOLERANCIA
            || tempo < res
            || !matches!(status, Some("WON" | "LOST"))
            || !g.resolucao_binaria()
        {
            return Ok(None);
        }
        if resgate.get("amountFilled").is_some_and(|v| !v.is_null()) && (unidades(resgate.get("amountFilled"), 18)? - e.cotas).abs() > TOLERANCIA {
            return Ok(None);
        }
        let cotas = e.cotas;
        vender(g, &mut e, &mut fechadas, "resgate", tempo, cotas, if status == Some("WON") { cotas } else { 0.0 });
    } else if (e.cotas - restante).abs() > TOLERANCIA {
        return Ok(None);
    }
    if e.cotas > TOLERANCIA && g.mercado.get("status").and_then(Value::as_str) == Some("RESOLVED") {
        let status = g.status_mantido();
        match resolucao {
            Some(res) if status == Some("LOST") && g.resolucao_binaria() => {
                let cotas = e.cotas;
                vender(g, &mut e, &mut fechadas, "resgate", res, cotas, 0.0);
            }
            _ if status != Some("WON") => return Ok(None),
            _ => {}
        }
    }
    Ok(Some(fechadas))
}

regex_js!(PS_VENCE_SIMPLES, r"(?i)^Will .+? win(?: on \d{4}-\d{2}-\d{2})?\?$");
regex_js!(PS_VS, r"(?i)\s+vs\.?\s+");

#[derive(Default)]
struct Montagem {
    grupos: IndexMap<String, Grupo>,
    esportes: HashSet<String>,
    resolvidos: IndexSet<String>,
}

impl Montagem {
    /// `groupFor()`.
    fn grupo_para(&mut self, m: Option<&Value>, o: Option<&Value>) -> Result<String> {
        let (ident, id, indice) = identidade(m, o)?;
        let (m, o) = (m.unwrap_or(&Value::Null), o.unwrap_or(&Value::Null));
        let condicao = n_texto(id);
        let existe = self.grupos.contains_key(&ident);
        let linha = if existe { None } else { Some(linha_predict(m, o, id, indice)) };
        if !self.esportes.contains(&condicao)
            && (e_esporte(Some(m))
                || predict_categoria(&[Some(m)]) != "Outros"
                || PS_VENCE_SIMPLES.is_match(&texto(m.get("question")))
                || PS_VS.is_match(&texto(m.get("title"))))
        {
            self.esportes.insert(condicao.clone());
        }
        let resolvido = m.get("status").and_then(Value::as_str) == Some("RESOLVED");
        if resolvido {
            self.resolvidos.insert(condicao);
        }
        if let Some(linha) = linha {
            self.grupos.insert(ident.clone(), Grupo { linha, mercado: m.clone(), preenchimentos: IndexMap::new(), resgates: Vec::new() });
        }
        if resolvido {
            if let Some(g) = self.grupos.get_mut(&ident) {
                g.mercado = m.clone();
            }
        }
        Ok(ident)
    }
}

struct Foto {
    abertas: Vec<Value>,
    fechadas: Vec<Value>,
    aviso: String,
}

/// `predictSnapshot()`.
async fn foto_predict(estado: &EstadoTracked, chave: &str, carteira: &str) -> Result<Foto> {
    let api = Predict { chave: chave.to_string() };
    let (caminho_posicoes, caminho_atividade) = (format!("/positions/{carteira}"), format!("/account/{carteira}/activity"));
    let (posicoes, negocios, atividade) = futures::try_join!(
        api.todas(&caminho_posicoes, vec![("first", "100".into()), ("includeLost", "true".into())]),
        api.todas("/orders/matches", vec![("first", "150".into()), ("signerAddress", carteira.to_string())]),
        api.todas(&caminho_atividade, vec![("first", "150".into())])
    )?;
    let mut mt = Montagem::default();
    let mut mantidas: HashMap<String, (Map<String, Value>, f64)> = HashMap::new();
    for p in &posicoes {
        if p.is_null() {
            return Err(invalida());
        }
        let ident = mt.grupo_para(p.get("market"), p.get("outcome"))?;
        let tamanho = unidades(p.get("amount"), 18)?;
        if tamanho <= 0.0 {
            continue;
        }
        let media = numero(p.get("averageBuyPriceUsd"))?;
        let mut h = mt.grupos[&ident].linha.clone();
        h.insert("id".into(), json!(ident));
        h.insert("size".into(), n_json(tamanho));
        h.insert("avgPrice".into(), n_json(media));
        h.insert("initialValue".into(), n_json(tamanho * media));
        h.insert("currentValue".into(), n_json(numero(p.get("valueUsd"))?));
        h.insert("cashPnl".into(), n_json(numero(p.get("pnlUsd"))?));
        mantidas.insert(ident, (h, tamanho));
    }
    for n in &negocios {
        let tomador = n.get("taker");
        let criadores = n.get("makers").and_then(Value::as_array);
        let (true, true, Some(criadores)) = (js::truthy(n.get("transactionHash")), js::truthy(tomador), criadores) else {
            return Err(invalida());
        };
        for (i, perna) in tomador.into_iter().chain(criadores.iter()).enumerate() {
            match perna.get("signer") {
                _ if perna.is_null() => return Err(invalida()),
                None | Some(Value::Null) => continue,
                Some(Value::String(s)) if s.to_lowercase() == carteira => {}
                Some(Value::String(_)) => continue,
                Some(_) => return Err(invalida()),
            }
            let ident = mt.grupo_para(n.get("market"), perna.get("outcome"))?;
            let p = preenchimento(n, perna, i == 0)?;
            if !js::truthy(perna.get("hash")) {
                return Err(invalida());
            }
            if let Some(g) = mt.grupos.get_mut(&ident) {
                g.preenchimentos.insert(p.id.clone(), p);
            }
        }
    }
    let mut vistos: HashSet<String> = HashSet::new();
    let mut ambiguos: HashSet<String> = HashSet::new();
    let mut categorias_ambiguas: HashSet<String> = HashSet::new();
    for ev in &atividade {
        let mercado = ev.get("market");
        if !js::truthy(mercado.and_then(|m| m.get("id"))) {
            return Err(invalida());
        }
        let nome = ev.get("name").and_then(Value::as_str);
        if nome == Some("REDEEM") && js::truthy(ev.get("outcome")) && js::truthy(ev.get("transactionHash")) {
            let ident = mt.grupo_para(mercado, ev.get("outcome"))?;
            let id = format!("{ident}:{}", js::string(ev.get("transactionHash")));
            if vistos.insert(id) {
                if let Some(g) = mt.grupos.get_mut(&ident) {
                    g.resgates.push(ev.clone());
                }
            }
        } else {
            ambiguos.insert(js::string(mercado.and_then(|m| m.get("id"))));
            let slug = mercado.and_then(|m| m.get("categorySlug"));
            if nome == Some("CONVERT") && js::truthy(slug) {
                categorias_ambiguas.insert(js::string(slug));
            }
        }
    }
    let ids: Vec<String> = mt.resolvidos.iter().cloned().collect();
    let mut resolucoes: HashMap<String, Option<f64>> = HashMap::new();
    for lote in ids.chunks(100) {
        let r = api.api("/markets/status-logs", &vec![("ids", lote.join(","))], PRAZO_PADRAO).await?;
        let Some(Value::Array(dados)) = r.get("data") else { return Err(invalida()) };
        for m in dados {
            let Some(Value::Array(logs)) = m.get("logs") else { return Err(invalida()) };
            if let Some(l) = logs.iter().find(|l| l.get("status").and_then(Value::as_str) == Some("RESOLVED")) {
                resolucoes.insert(js::string(m.get("marketId")), audit::timestamp_ms(l.get("timestamp")));
            }
        }
    }
    let mut abertas: Vec<Map<String, Value>> = Vec::new();
    let mut fechadas: Vec<Map<String, Value>> = Vec::new();
    let mut incompleto = !ambiguos.is_empty();
    for (ident, g) in &mt.grupos {
        let mantida = mantidas.get(ident);
        let condicao = js::string(g.linha.get("conditionId"));
        let slug = js::string(g.linha.get("_marketSlug"));
        let ambiguo = ambiguos.contains(&condicao) || categorias_ambiguas.contains(&slug);
        let resultados = if ambiguo { None } else { conciliar(g, mantida.map(|m| m.1), resolucoes.get(&condicao).copied().flatten())? };
        match resultados {
            None => incompleto = true,
            Some(rs) => {
                for mut r in rs {
                    let id = format!("{ident}:{}", js::string(r.get("id")));
                    r.insert("id".into(), json!(id));
                    fechadas.push(r);
                }
            }
        }
        if let Some((h, _)) = mantida {
            if g.mercado.get("status").and_then(Value::as_str) != Some("RESOLVED") || g.status_mantido() == Some("WON") {
                abertas.push(h.clone());
            }
        }
    }
    enriquecer(estado, &api, &mut abertas, &mut fechadas, &mt.esportes).await;
    Ok(Foto {
        abertas: abertas.into_iter().map(Value::Object).collect(),
        fechadas: fechadas.into_iter().map(Value::Object).collect(),
        aviso: if incompleto {
            "O Predict.fun retornou operações que não puderam ser conciliadas. O histórico realizado pode estar incompleto.".into()
        } else {
            String::new()
        },
    })
}

// ---------------------------------------------------------------------------
// Predict.fun: agenda das categorias (data do jogo, título e seleção)
// ---------------------------------------------------------------------------

regex_js!(AG_VS_PONTO, r"(?i)\s+vs\.\s+");
regex_js!(AG_APELIDO, r"^[A-Z0-9]{2,6}$");

/// `predictSchedule()`.
async fn buscar_agenda(estado: &EstadoTracked, api: &Predict, slug: &str, prazo: Duration) -> Result<Arc<Agenda>> {
    let mut r = api.api(&format!("/categories/{}", net::codificar(slug)), &vec![], prazo).await?;
    let d = tirar(&mut r, "data").unwrap_or(Value::Null);
    let Some(lista) = d.get("markets").and_then(Value::as_array).filter(|_| d.get("slug").and_then(Value::as_str) == Some(slug)) else {
        return Err(invalida());
    };
    let inicio = if e_esporte(Some(&d)) { iso_date(d.get("startsAt")) } else { String::new() };
    let bruto = match d.get("title") {
        Some(Value::String(t)) => AG_VS_PONTO.replace(t.trim(), " vs ").into_owned(),
        _ => String::new(),
    };
    let titulo = titles::clean_kalshi_matchup(&bruto);
    let escopo = if titulo != bruto { bruto.clone() } else { String::new() };
    let mut mercados = IndexMap::new();
    for m in lista {
        let Some(id) = inteiro_seguro(m.get("id")) else { continue };
        let categoria = predict_categoria(&[Some(m), Some(&d)]);
        let data = if !inicio.is_empty() {
            inicio.clone()
        } else if e_esporte(Some(m)) {
            iso_date(m.get("speedBump").and_then(|s| s.get("liveFrom")))
        } else {
            String::new()
        };
        let resultados = match m.get("outcomes") {
            None | Some(Value::Null) => Vec::new(),
            Some(v) if !js::truthy(Some(v)) => Vec::new(),
            Some(Value::Array(a)) => a
                .iter()
                .filter(|o| inteiro(o.get("indexSet")).is_some() && o.get("name").is_some_and(Value::is_string))
                .map(|o| {
                    let mut s = Map::new();
                    for k in ["indexSet", "name", "variantDetails", "team"] {
                        if let Some(v) = o.get(k) {
                            s.insert(k.into(), v.clone());
                        }
                    }
                    Value::Object(s)
                })
                .collect(),
            Some(_) => return Err(invalida()),
        };
        let mut sub = Map::new();
        for k in ["id", "question", "marketType", "marketVariant", "variantDetails", "team"] {
            if let Some(v) = m.get(k) {
                sub.insert(k.into(), v.clone());
            }
        }
        sub.insert("outcomes".into(), Value::Array(resultados));
        mercados.insert(n_texto(id), MetaMercado { data, categoria, mercado: Value::Object(sub) });
    }
    let validade = if !inicio.is_empty() && d.get("status").and_then(Value::as_str) == Some("RESOLVED") { 24.0 * 3_600_000.0 } else { 300_000.0 };
    let agenda = Arc::new(Agenda { titulo, escopo, mercados, expira: agora_ms() + validade });
    if let Ok(mut c) = estado.agendas.lock() {
        c.shift_remove(slug);
        c.insert(slug.to_string(), agenda.clone());
        if c.len() > 4096 {
            c.shift_remove_index(0);
        }
    }
    Ok(agenda)
}

fn aplicar_agenda(linhas: &mut [&mut Map<String, Value>], agenda: Option<&Agenda>) {
    let Some(ag) = agenda else { return };
    let generica = |c: &str| c == "Outros" || c == "Outros Esportes";
    for row in linhas.iter_mut() {
        let condicao = js::string(row.get("conditionId"));
        let Some(meta) = ag.mercados.get(&condicao) else { continue };
        if !meta.data.is_empty() && !js::truthy(row.get("_startDate")) {
            row.insert("_startDate".into(), json!(meta.data));
        }
        let atual = row.get("_category").and_then(Value::as_str).unwrap_or("");
        if !generica(meta.categoria) || generica(atual) {
            row.insert("_category".into(), json!(meta.categoria));
        }
        let escolhido = meta
            .mercado
            .get("outcomes")
            .and_then(Value::as_array)
            .and_then(|l| l.iter().find(|o| mesmo_valor(o.get("indexSet"), row.get("outcomeIndex"))));
        if let Some(escolhido) = escolhido {
            // A posição pode chamar o lado de "Bills" e a categoria de "BUF": fica o nome mantido.
            let mut mercado = meta.mercado.clone();
            if let Some(o) = mercado.as_object_mut() {
                let tipo = if js::truthy(o.get("marketType")) {
                    o.get("marketType").cloned().unwrap_or(Value::Null)
                } else {
                    row.get("_predictMarketType").cloned().unwrap_or(Value::Null)
                };
                o.insert("marketType".into(), tipo);
                o.insert("_predictScope".into(), json!(ag.escopo));
            }
            let time = titles::ou(
                escolhido.get("variantDetails").and_then(|v| v.get("sports")).and_then(|s| s.get("team")),
                escolhido.get("team"),
            );
            let saida = js::string(row.get("outcome"));
            let candidatos: Vec<&str> = [row.get("outcome"), escolhido.get("name"), time.and_then(|t| t.get("name"))]
                .into_iter()
                .filter_map(|v| v.and_then(Value::as_str))
                .filter(|s| !s.trim().is_empty())
                .collect();
            let mantido = if AG_APELIDO.is_match(&saida) {
                candidatos.iter().find(|n| !AG_APELIDO.is_match(n)).map_or(saida.clone(), |n| n.to_string())
            } else {
                saida.clone()
            };
            let mut resultado = escolhido.clone();
            if let Some(o) = resultado.as_object_mut() {
                o.insert("name".into(), json!(mantido));
            }
            let categoria = row.get("_category").and_then(Value::as_str).unwrap_or("").to_string();
            row.insert("_predictSelection".into(), json!(predict_selecao(&mercado, &resultado, &categoria)));
        }
        let selecao = titles::kalshi_bet_qualifier(&js::string(row.get("_predictSelection")), &[ag.escopo.as_str()]);
        row.insert("_predictSelection".into(), json!(selecao));
        if !ag.titulo.is_empty() && !selecao.is_empty() {
            row.insert("_predictTitle".into(), json!(format!("{} - {selecao}", ag.titulo)));
        }
    }
}

/// `enrichPredictSchedule()`: primeiro a visão ao vivo; no máximo 48 consultas
/// e 7,5 s no total.
async fn enriquecer(
    estado: &EstadoTracked,
    api: &Predict,
    abertas: &mut [Map<String, Value>],
    fechadas: &mut [Map<String, Value>],
    esportes: &HashSet<String>,
) {
    let realizado = |m: &Map<String, Value>| js::number(m.get("_realizedAt"));
    let mut fech: Vec<usize> = (0..fechadas.len()).filter(|&i| esportes.contains(&js::string(fechadas[i].get("conditionId")))).collect();
    fech.sort_by(|&a, &b| realizado(&fechadas[b]).partial_cmp(&realizado(&fechadas[a])).unwrap_or(std::cmp::Ordering::Equal));
    let ordem: Vec<(bool, usize)> = (0..abertas.len()).map(|i| (true, i)).chain(fech.into_iter().map(|i| (false, i))).collect();
    let mut por_slug: IndexMap<String, Vec<(bool, usize)>> = IndexMap::new();
    for (aberta, i) in ordem {
        let linha = if aberta { &abertas[i] } else { &fechadas[i] };
        if !js::truthy(linha.get("_marketSlug")) {
            continue;
        }
        por_slug.entry(js::string(linha.get("_marketSlug"))).or_default().push((aberta, i));
    }
    let condicao = |abertas: &[Map<String, Value>], fechadas: &[Map<String, Value>], (aberta, i): (bool, usize)| {
        js::string(if aberta { abertas[i].get("conditionId") } else { fechadas[i].get("conditionId") })
    };
    let mut aplicar: Vec<(Vec<(bool, usize)>, Arc<Agenda>)> = Vec::new();
    let mut faltando: Vec<(String, Vec<(bool, usize)>, Option<Arc<Agenda>>)> = Vec::new();
    let agora = agora_ms();
    for (slug, linhas) in por_slug {
        let cache = estado.agendas.lock().ok().and_then(|c| c.get(&slug).cloned());
        match cache {
            Some(c) if c.expira > agora && linhas.iter().all(|&l| c.mercados.contains_key(&condicao(abertas, fechadas, l))) => {
                aplicar.push((linhas, c))
            }
            outro => faltando.push((slug, linhas, outro)),
        }
    }
    faltando.truncate(48);
    let limite = Instant::now() + Duration::from_millis(7500);
    let consultados: Vec<(Vec<(bool, usize)>, Option<Arc<Agenda>>)> = futures::stream::iter(faltando)
        .map(|(slug, linhas, cache)| async move {
            let agora = Instant::now();
            if agora >= limite {
                return None;
            }
            let prazo = Duration::from_millis((limite - agora).as_millis().clamp(1, 4000) as u64);
            Some(match buscar_agenda(estado, api, &slug, prazo).await {
                Ok(a) => (linhas, Some(a)),
                Err(_) => (linhas, cache),
            })
        })
        .buffer_unordered(8)
        .filter_map(futures::future::ready)
        .collect()
        .await;
    let lotes = aplicar.into_iter().map(|(l, a)| (l, Some(a))).chain(consultados);
    // Cada linha pertence a um só slug: os lotes não se sobrepõem.
    let mut ab: Vec<Option<&mut Map<String, Value>>> = abertas.iter_mut().map(Some).collect();
    let mut fe: Vec<Option<&mut Map<String, Value>>> = fechadas.iter_mut().map(Some).collect();
    for (linhas, agenda) in lotes {
        let mut refs: Vec<&mut Map<String, Value>> = Vec::with_capacity(linhas.len());
        for (aberta, i) in linhas {
            let alvo = if aberta { ab[i].take() } else { fe[i].take() };
            if let Some(r) = alvo {
                refs.push(r);
            }
        }
        aplicar_agenda(&mut refs, agenda.as_deref());
    }
}

// ---------------------------------------------------------------------------
// Rotas
// ---------------------------------------------------------------------------

fn resposta(status: StatusCode, body: Value) -> ApiResponse {
    ApiResponse { status, body, bruto: None }
}

fn falha(status: StatusCode, mensagem: &str) -> ApiResponse {
    resposta(status, json!({ "ok": false, "error": mensagem }))
}

fn desconhecida() -> ApiResponse {
    falha(StatusCode::NOT_FOUND, "Casa desconhecida.")
}

/// `trackedConnection()`: `(account_scope, credentials_enc)`.
async fn ler_conexao(estado: &Estado, casa: &str) -> Result<Option<(String, Option<String>)>> {
    let (u, p) = (estado.usuario.id.clone(), casa.to_string());
    estado
        .db
        .ler(move |c| {
            Ok(c.query_row(
                "SELECT account_scope, credentials_enc FROM tracked_connections WHERE user_id=? AND platform=?",
                params![u, p],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
        })
        .await
}

async fn cifrada_atual(estado: &Estado, casa: &str) -> Result<Option<String>> {
    Ok(ler_conexao(estado, casa).await?.and_then(|c| c.1))
}

struct Credenciais {
    chave: zeroize::Zeroizing<String>,
    carteira: String,
}

fn credenciais(estado: &Estado, cifrada: &str) -> Result<Credenciais> {
    let texto = estado.chave.decrypt(cifrada)?;
    let v: Value = serde_json::from_str(&texto)?;
    Ok(Credenciais { chave: zeroize::Zeroizing::new(js::string(v.get("apiKey"))), carteira: js::string(v.get("wallet")) })
}

async fn validar(casa: &str, cred: &Credenciais) -> Result<String> {
    if casa == "predict" { validar_predict(&cred.chave, &cred.carteira).await } else { validar_sx(&cred.chave).await }
}

/// `POST /api/connections/:platform`.
pub async fn conectar(estado: &Estado, casa: &str, corpo: &Value) -> Result<ApiResponse> {
    if !casa_valida(casa) {
        return Ok(desconhecida());
    }
    let chave = corpo.get("apiKey").and_then(Value::as_str).map_or(String::new(), |s| s.trim().to_string());
    let carteira = corpo.get("wallet").and_then(Value::as_str).map_or(String::new(), |s| s.trim().to_lowercase());
    if chave.is_empty() || chave.encode_utf16().count() > 1024 || !chave.bytes().all(|b| (b'!'..=b'~').contains(&b)) {
        return Ok(falha(StatusCode::BAD_REQUEST, "Informe uma chave de API válida."));
    }
    let carteira_ok = carteira.len() == 42 && carteira.starts_with("0x") && carteira[2..].bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    if casa == "predict" && !carteira_ok {
        return Ok(falha(StatusCode::BAD_REQUEST, "Informe o endereço público da carteira da conta Predict.fun."));
    }
    let resultado: Result<ApiResponse> = async {
        let anterior = cifrada_atual(estado, casa).await?;
        let cred = Credenciais { chave: zeroize::Zeroizing::new(chave.clone()), carteira: carteira.clone() };
        let conta = validar(casa, &cred).await?;
        let escopo = estado.chave.hmac_hex(&js::para_texto(&json!([estado.usuario.id, casa, conta])));
        if cifrada_atual(estado, casa).await? != anterior {
            return Ok(falha(StatusCode::CONFLICT, "A conexão mudou durante a validação. Tente novamente."));
        }
        let json_cred =
            zeroize::Zeroizing::new(if casa == "predict" { js::para_texto(&json!({ "apiKey": chave, "wallet": carteira })) } else { js::para_texto(&json!({ "apiKey": chave })) });
        let cifrada = estado.chave.encrypt(&json_cred)?;
        let (u, p) = (estado.usuario.id.clone(), casa.to_string());
        estado
            .db
            .call(move |c| {
                c.execute(
                    "INSERT INTO tracked_connections(user_id,platform,account_scope,credentials_enc) VALUES(?,?,?,?)
      ON CONFLICT(user_id,platform) DO UPDATE SET account_scope=excluded.account_scope,credentials_enc=excluded.credentials_enc",
                    params![u, p, escopo, cifrada],
                )?;
                Ok(())
            })
            .await?;
        Ok(resposta(StatusCode::OK, json!({ "ok": true })))
    }
    .await;
    Ok(resultado.unwrap_or_else(|e| match net::status_do_erro(&e) {
        Some(401 | 403) => falha(StatusCode::BAD_REQUEST, "A API recusou a chave ou suas permissões."),
        _ => {
            eprintln!("[{casa}] conexão: {e:#}");
            falha(StatusCode::BAD_GATEWAY, "Não foi possível validar a conta na plataforma.")
        }
    }))
}

/// `GET /api/connections/:platform/test`.
pub async fn testar(estado: &Estado, casa: &str) -> ApiResponse {
    if !casa_valida(casa) {
        return desconhecida();
    }
    let Ok(Some(cifrada)) = cifrada_atual(estado, casa).await else {
        return falha(StatusCode::CONFLICT, "Conecte a casa primeiro.");
    };
    let r: Result<bool> = async {
        validar(casa, &credenciais(estado, &cifrada)?).await?;
        Ok(cifrada_atual(estado, casa).await?.as_deref() == Some(cifrada.as_str()))
    }
    .await;
    match r {
        Ok(true) => resposta(StatusCode::OK, json!({ "ok": true })),
        Ok(false) => falha(StatusCode::CONFLICT, "A conexão mudou durante o teste. Tente novamente."),
        Err(e) => {
            eprintln!("[{casa}] teste: {e:#}");
            falha(StatusCode::BAD_GATEWAY, "Não foi possível validar a conexão com a plataforma.")
        }
    }
}

/// `DELETE /api/connections/:platform`: apaga só as credenciais; o histórico fica.
pub async fn desconectar(estado: &Estado, casa: &str) -> Result<ApiResponse> {
    if !casa_valida(casa) {
        return Ok(desconhecida());
    }
    let (u, p) = (estado.usuario.id.clone(), casa.to_string());
    estado
        .db
        .call(move |c| Ok(c.execute("UPDATE tracked_connections SET credentials_enc=NULL WHERE user_id=? AND platform=?", params![u, p])?))
        .await?;
    Ok(resposta(StatusCode::OK, json!({ "ok": true })))
}

/// Entrada do índice para uma aposta gravada (`persistTrackedResults`).
fn entrada_indice(row: &Value) -> Value {
    let inicio = audit::timestamp_ms(row.get("_startDate"));
    let predict = row.get("_platform").and_then(Value::as_str) == Some("predict");
    let data_stake = (if predict { None } else { inicio }).filter(|v| *v != 0.0).or_else(|| audit::timestamp_ms(row.get("timestamp")));
    let ymd = |v: Option<f64>| v.filter(|v| *v != 0.0).map_or(String::new(), ymd_sao_paulo);
    let titulo = titles::ou(row.get("_predictTitle"), row.get("title")).cloned().unwrap_or(Value::Null);
    let assinatura = hex::encode(ring::digest::digest(&ring::digest::SHA256, js::para_texto(row).as_bytes()));
    json!({
        "platform": row.get("_platform"),
        "betKey": row.get("slug"),
        "category": row.get("_category"),
        "startYmd": ymd(inicio),
        "tableYmd": ymd(data_stake),
        "tableTs": data_stake.map_or(Value::Null, n_json),
        "pnlValue": row.get("realizedPnl"),
        "stakeValue": n_json(stake_historico(row)),
        "oddValue": n_json(odd_historica(row)),
        "won": i64::from(js::number(row.get("realizedPnl")) > 0.0),
        "titleSearch": titulo,
        "titleDisplay": titulo,
        "sourceSignature": assinatura
    })
}

/// `persistTrackedResults()`: ledger e índice numa transação só.
fn persistir(c: &mut Connection, user_id: &str, escopo: &str, fechadas: &[Value]) -> Result<()> {
    let linhas: Vec<Value> = fechadas
        .iter()
        .map(|r| {
            let mut o = r.as_object().cloned().unwrap_or_default();
            o.insert("slug".into(), json!(format!("{escopo}:{}", js::string(r.get("id")))));
            Value::Object(o)
        })
        .collect();
    fn gravar(c: &mut Connection, user_id: &str, linhas: Vec<Value>) -> Result<()> {
        let g = ledger::gravar(c, user_id, linhas, true)?;
        let entradas: Vec<Value> = g.alteradas.iter().map(|&i| entrada_indice(&g.processadas[i])).collect();
        ledger::salvar_indice(c, user_id, Some(&Value::Array(entradas)))?;
        Ok(())
    }
    c.execute_batch("BEGIN")?;
    match gravar(c, user_id, linhas) {
        Ok(()) => Ok(c.execute_batch("COMMIT")?),
        Err(e) => {
            let _ = c.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

/// `POST /api/tracked/:platform/sync`.
pub async fn sincronizar(estado: &Estado, casa: &str) -> ApiResponse {
    if !casa_valida(casa) {
        return desconhecida();
    }
    let conexao = match ler_conexao(estado, casa).await {
        Ok(Some((escopo, Some(cifrada)))) => (escopo, cifrada),
        _ => return falha(StatusCode::CONFLICT, "Conta não conectada."),
    };
    let (escopo, cifrada) = conexao;
    let r: Result<Option<Value>> = async {
        let cred = credenciais(estado, &cifrada)?;
        let foto = if casa == "predict" {
            foto_predict(&estado.tracked, &cred.chave, &cred.carteira).await?
        } else {
            foto_sx(&cred.chave).await?
        };
        if cifrada_atual(estado, casa).await?.as_deref() != Some(cifrada.as_str()) {
            return Ok(None);
        }
        let (u, fechadas) = (estado.usuario.id.clone(), foto.fechadas.clone());
        estado.db.call(move |c| persistir(c, &u, &escopo, &fechadas)).await?;
        let limite = agora_ms() - 2.0 * 60.0 * 60.0 * 1000.0;
        let recentes: Vec<&Value> =
            foto.fechadas.iter().filter(|r| audit::timestamp_ms(r.get("_startDate")).is_some_and(|t| t >= limite)).collect();
        Ok(Some(json!({
            "ok": true,
            "open": foto.abertas,
            "recent": recentes,
            "closedCount": foto.fechadas.len(),
            "warning": foto.aviso
        })))
    }
    .await;
    match r {
        Ok(Some(corpo)) => resposta(StatusCode::OK, corpo),
        Ok(None) => falha(StatusCode::CONFLICT, "A conexão mudou durante a consulta."),
        Err(e) => {
            eprintln!("[{casa}] sincronização: {e:#}");
            let msg = if net::status_do_erro(&e) == Some(429) {
                "A API atingiu o limite de consultas. Tente novamente em instantes."
            } else {
                "Não foi possível sincronizar com a plataforma."
            };
            falha(StatusCode::BAD_GATEWAY, msg)
        }
    }
}

/// `GET /api/tracked/predict/balance`: USDT da carteira na BNB Chain.
pub async fn saldo_predict(estado: &Estado) -> ApiResponse {
    let Ok(Some(cifrada)) = cifrada_atual(estado, "predict").await else {
        return falha(StatusCode::CONFLICT, "Conta Predict.fun não conectada.");
    };
    let r: Result<f64> = async {
        let carteira = credenciais(estado, &cifrada)?.carteira;
        poly::rustls_pronto();
        for rpc in BNB_RPCS {
            if let Ok(v) = poly::saldo_erc20(&carteira, rpc, PREDICT_USDT, 18, Duration::from_millis(4500)).await {
                return Ok(v);
            }
        }
        bail!("Saldo indisponível.")
    }
    .await;
    match r {
        Ok(v) => {
            if cifrada_atual(estado, "predict").await.ok().flatten().as_deref() != Some(cifrada.as_str()) {
                return falha(StatusCode::CONFLICT, "A conexão mudou durante a consulta.");
            }
            resposta(StatusCode::OK, json!({ "ok": true, "balance": n_json(v) }))
        }
        Err(e) => {
            eprintln!("[predict] saldo: {e:#}");
            falha(StatusCode::BAD_GATEWAY, "Não foi possível consultar o saldo em USDT do Predict.fun.")
        }
    }
}

/// `GET /api/tracked/sxbet/balance`.
pub async fn saldo_sxbet(estado: &Estado) -> ApiResponse {
    let Ok(Some(cifrada)) = cifrada_atual(estado, "sxbet").await else {
        return falha(StatusCode::CONFLICT, "Conta SX Bet não conectada.");
    };
    let r: Result<(f64, f64)> = async { saldo_sx(&credenciais(estado, &cifrada)?.chave).await }.await;
    match r {
        Ok((saldo, travado)) => {
            if cifrada_atual(estado, "sxbet").await.ok().flatten().as_deref() != Some(cifrada.as_str()) {
                return falha(StatusCode::CONFLICT, "A conexão mudou durante a consulta.");
            }
            resposta(StatusCode::OK, json!({ "ok": true, "balance": n_json(saldo), "escrowedAmount": n_json(travado) }))
        }
        Err(e) => match net::status_do_erro(&e) {
            Some(401 | 403) => falha(StatusCode::FORBIDDEN, "A SX Bet recusou a leitura do saldo com esta chave."),
            _ => {
                eprintln!("[sxbet] saldo: {e:#}");
                falha(StatusCode::BAD_GATEWAY, "Não foi possível consultar o saldo da SX Bet.")
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unidades_e_numeros_como_no_js() {
        assert_eq!(unidades(Some(&json!("1500000")), 6).unwrap(), 1.5);
        assert_eq!(unidades(Some(&json!("-25")), 1).unwrap(), -2.5);
        assert!(unidades(Some(&json!("1.5")), 6).is_err());
        assert!(unidades(Some(&json!(15)), 6).is_err());
        assert_eq!(numero(Some(&json!(" 0.25 "))).unwrap(), 0.25);
        assert!(numero(Some(&json!(""))).is_err());
        assert!(numero(Some(&json!(null))).is_err());
        assert_eq!(form("a b,c/é"), "a+b%2Cc%2F%C3%A9");
    }

    #[test]
    fn conciliacao_compra_venda_e_resgate() {
        let mercado = json!({ "id": 7, "title": "Lakers vs Celtics", "status": "RESOLVED", "resolution": { "indexSet": 1 },
            "outcomes": [{ "indexSet": 1, "name": "Lakers", "status": "WON" }, { "indexSet": 2, "name": "Celtics", "status": "LOST" }] });
        let resultado = json!({ "indexSet": 1, "name": "Lakers" });
        let p = |id: &str, compra: bool, tempo: f64, preco: f64, cotas: f64| Preenchimento {
            compra, tempo, preco, cotas, caixa: cotas * preco, id: id.into(),
        };
        let mut g = Grupo { linha: linha_predict(&mercado, &resultado, 7.0, 1.0), mercado, preenchimentos: IndexMap::new(), resgates: vec![] };
        g.preenchimentos.insert("a".into(), p("a", true, 1_700_000_000_000.0, 0.4, 10.0));
        g.preenchimentos.insert("b".into(), p("b", false, 1_700_000_100_000.0, 0.6, 4.0));
        g.resgates.push(json!({ "createdAt": 1_700_000_300_000i64 }));
        let r = conciliar(&g, None, Some(1_700_000_200_000.0)).unwrap().unwrap();
        assert_eq!(r.len(), 2);
        assert!((js::number(r[0].get("realizedPnl")) - 0.8).abs() < 1e-9);
        assert!((js::number(r[1].get("realizedPnl")) - 3.6).abs() < 1e-9);
        assert_eq!(r[1].get("id"), Some(&json!("resgate")));
        // Venda maior que a posição: não concilia.
        g.preenchimentos.insert("c".into(), p("c", false, 1_700_000_150_000.0, 0.5, 20.0));
        assert!(conciliar(&g, None, Some(1_700_000_200_000.0)).unwrap().is_none());
    }

    /// Compara `predictSelection`/`predictCategory` com o JS original
    /// (casos gerados por `predict-oracle.mjs`; env `BETTY_ORACULO_PREDICT`).
    #[test]
    #[ignore]
    fn oraculo_predict() {
        let arquivo = std::env::var("BETTY_ORACULO_PREDICT").expect("defina BETTY_ORACULO_PREDICT");
        let (mut total, mut erros) = (0, 0);
        for linha in std::fs::read_to_string(arquivo).unwrap().lines().filter(|l| !l.is_empty()) {
            let caso: Value = serde_json::from_str(linha).unwrap();
            let (m, o) = (&caso["m"], &caso["o"]);
            let pai = Some(&caso["parent"]).filter(|p| !p.is_null());
            let categoria = match caso["c"].as_str() {
                Some(c) => c.to_string(),
                None => predict_categoria(&[Some(m)]).to_string(),
            };
            let obtido = [predict_selecao(m, o, &categoria), predict_categoria(&[Some(m)]).into(), predict_categoria(&[Some(m), pai]).into()];
            let esperado = [caso["sel"].as_str().unwrap(), caso["cat1"].as_str().unwrap(), caso["cat2"].as_str().unwrap()];
            total += 1;
            if obtido.iter().zip(esperado).any(|(a, b)| a != b) {
                erros += 1;
                if erros <= 15 {
                    eprintln!("DIFERENTE\n  m={m}\n  o={o} c={}\n  js={esperado:?}\n  rs={obtido:?}", caso["c"]);
                }
            }
        }
        eprintln!("{total} casos, {erros} diferentes");
        assert_eq!(erros, 0);
    }
}
