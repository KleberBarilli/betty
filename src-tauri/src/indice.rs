//! Campos do índice de apostas fechadas, iguais a `indiceHistoricoFechado()`
//! do `dashboard.js` (com `chaveIndiceHistorico()`, `chaveLedgerFechada()` e
//! `ymdHistoricoDb()`) e aos auxiliares dela: `dataInicioAposta()` (com
//! `startFromKalshiTicker()`), `getTsResultadoPnL()` (com o `pnlTimestamp()`,
//! o `resolvedLoss()` e o `timestampMs()` do `audit-utils.js`), `getStake()`,
//! `getOddNum()`, `assinaturaFonteLedger()` (com `serializarLedgerEstavel()`)
//! e `versaoTituloConfirmado()`.
//!
//! Parte da sincronização no Rust (`docs/desempenho.md`): ainda não é usada
//! pelas rotas, e o oráculo (`tests::oraculo_ajudantes_indice`) compara com o
//! JS real.
//!
//! - O `pnlTimestamp()` daqui é o do JS, com exceção e `Date.parse` do V8
//!   (`gamma::data_js`). O `audit::pnl_timestamp` das rotas usa
//!   `js::data_ms`/`js::number` e não lança: os dois só diferem em
//!   plataforma que não é texto e em datas e números fora do que as APIs
//!   mandam.
//! - `metasDatasPnl` (o mapa global que a sincronização preenche) entra como
//!   parâmetro: `None` é o `{}` de `lerCacheMetasDatasGamma()`.
//! - As datas do `Intl.DateTimeFormat("en-CA")` em São Paulo saem como no ICU
//!   (ano sem zeros à esquerda, antes de Cristo sem a era); fora dos ±262.142
//!   anos do `chrono` ficam vazias.

use chrono::{Datelike, TimeZone, Utc};
use chrono_tz::America::Sao_Paulo;
use serde_json::{Value, json};

use crate::categorias::{self, Excecao};
use crate::titles::{self, Escopo, clean_v, ou};
use crate::{exibicao, gamma, js, regex_exata};

type R<T> = Result<T, Excecao>;

/// `VERSAO_INDICE_TITULO`.
pub const VERSAO_INDICE_TITULO: &str = "28";

fn verdadeiro(v: Option<&Value>) -> Option<&Value> {
    v.filter(|v| js::truthy(Some(v)))
}

fn nao_nulo(v: Option<&Value>) -> bool {
    v.is_some_and(|v| !v.is_null())
}

/// O que uma função do JS devolve quando pode ser um campo cru da aposta ou
/// um número calculado (`getStake()`).
#[derive(Debug, Clone, PartialEq)]
pub enum ValorJs {
    Bruto(Value),
    Numero(f64),
}

impl ValorJs {
    /// `Number(valor)`.
    pub fn numero(&self) -> f64 {
        match self {
            ValorJs::Bruto(v) => js::numero(Some(v)),
            ValorJs::Numero(n) => *n,
        }
    }
}

regex_exata!(SERIE_EUA, r"KXMLB|KXNBA|KXNFL|KXNHL|KXNWSL|KXMLS|KXWNBA|KXNCAAF|KXNCAAB");
regex_exata!(TICKER_COM_DATA, r"(?i)(\d{2})(JAN|FEB|MAR|APR|MAY|JUN|JUL|AUG|SEP|OCT|NOV|DEC)(\d{2})(\d{2})(\d{2})");
const MESES: [&str; 12] = ["JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC"];

/// `startFromKalshiTicker(ticker)`: data e hora (ET) que as séries dos EUA
/// codificam no ticker (`KXMLBTEAMTOTAL-26AUG081805TORPHI` = 08/08 18:05 ET).
fn start_from_kalshi_ticker(ticker: Option<&Value>) -> Option<f64> {
    let texto = js::texto(Some(verdadeiro(ticker)?));
    if !SERIE_EUA.is_match(&texto.to_uppercase()) {
        return None;
    }
    let m = TICKER_COM_DATA.captures(&texto)?;
    let mes = MESES.iter().position(|x| *x == m[2].to_uppercase())?;
    let (dia, hora, minuto): (u32, u32, u32) = (m[3].parse().ok()?, m[4].parse().ok()?, m[5].parse().ok()?);
    if hora > 23 || minuto > 59 {
        return None;
    }
    let fuso = if (2..=10).contains(&mes) { "-04:00" } else { "-05:00" };
    let ano = 2000 + m[1].parse::<u32>().ok()?;
    gamma::data_js(&format!("{ano}-{:02}-{dia:02}T{hora:02}:{minuto:02}:00{fuso}", mes + 1))
}

/// `(d._platform || "polymarket").toLowerCase()`: exceção se não for texto.
fn plataforma_minuscula(d: &Value) -> R<String> {
    if d.is_null() {
        return Err(Excecao);
    }
    match verdadeiro(d.get("_platform")) {
        None => Ok("polymarket".into()),
        Some(Value::String(s)) => Ok(s.to_lowercase()),
        Some(_) => Err(Excecao),
    }
}

/// `dataInicioAposta(d)` em milissegundos. A Kalshi usa só a data oficial e,
/// sem ela, a do ticker das séries dos EUA.
pub fn data_inicio_aposta(d: &Value) -> R<Option<f64>> {
    if plataforma_minuscula(d)? == "kalshi" {
        if let Some(t) = gamma::parse_api_date(d.get("_startDate")) {
            return Ok(Some(t));
        }
        return Ok(start_from_kalshi_ticker(ou(ou(d.get("ticker"), d.get("slug")), d.get("eventSlug"))));
    }
    Ok(gamma::parse_api_date(ou(d.get("_startDate"), d.get("endDate"))))
}

/// `timestampMs(value)` do `audit-utils.js`: segundos ou ms (ou texto de data
/// lido pelo `Date.parse`) entre 2000 e 2100.
pub fn timestamp_ms(v: Option<&Value>) -> Option<f64> {
    let v = v.filter(|v| !v.is_null() && v.as_str() != Some(""))?;
    let n = js::numero(Some(v));
    let ms = if n.is_finite() { if n.abs() < 1e12 { n * 1000.0 } else { n } } else { gamma::data_js(&js::texto(Some(v)))? };
    if !ms.is_finite() || ms.abs() > 8.64e15 {
        return None;
    }
    let ano = Utc.timestamp_millis_opt(ms.trunc() as i64).single()?.year();
    (2000..=2100).contains(&ano).then_some(ms)
}

/// `resolvedLoss(d)`: perda resolvida da Polymarket ainda não resgatada.
fn resolved_loss(d: &Value) -> R<bool> {
    Ok(plataforma_minuscula(d)? == "polymarket"
        && d.get("redeemable") == Some(&Value::Bool(true))
        && nao_nulo(d.get("curPrice"))
        && js::numero(d.get("curPrice")) == 0.0)
}

/// `pnlTimestamp(d, marketResolution)`: instante em que o resultado foi
/// realizado (0 = desconhecido).
pub fn pnl_timestamp(d: &Value, resolucao_mercado: Option<&Value>) -> R<f64> {
    let campo = |k: &str| d.get(k);
    if let Some(c) = timestamp_ms(campo("_confirmedClosedTs")) {
        return Ok(c);
    }
    if matches!(campo("_platform").and_then(Value::as_str), Some("predict" | "sxbet")) {
        return Ok(timestamp_ms(campo("_realizedAt")).unwrap_or(0.0));
    }
    if titles::plataforma(d, "polymarket") == "kalshi" {
        return Ok(timestamp_ms(ou(ou(campo("_settledTime"), campo("settled_time")), campo("settlement_time")))
            .or_else(|| timestamp_ms(ou(campo("_saleTs"), campo("_lastTradeTs"))))
            .unwrap_or(0.0));
    }
    let resolucao = timestamp_ms(ou(campo("_resolvedTime"), resolucao_mercado));
    // Uma perda resolvida não resgatada ainda carrega a data da compra.
    if resolved_loss(d)? {
        return Ok(resolucao.unwrap_or(0.0));
    }
    let fechada = timestamp_ms(ou(ou(campo("_saleTs"), campo("_lastTradeTs")), campo("timestamp")));
    // Venda antes da resolução: a venda. Resgate depois: a resolução.
    Ok(match (fechada, resolucao) {
        (Some(f), Some(r)) => f.min(r),
        (f, r) => f.or(r).unwrap_or(0.0),
    })
}

/// `getTsResultadoPnL(d)`: só o mercado exato dá a data de resolução, nunca o
/// evento, um mercado irmão ou o início do jogo.
pub fn get_ts_resultado_pnl(d: &Value, metas: Option<&Value>) -> R<f64> {
    let usa_metas = titles::plataforma(d, "polymarket") == "polymarket" && !js::truthy(d.get("_resolvedTime"));
    let mercado = |evento: Option<&Value>| -> Option<Value> {
        let m = js::propriedade(metas?, &js::texto(evento))?;
        let mercados = m.get("markets").filter(|v| !v.is_null())?;
        js::propriedade(mercados, &js::texto(d.get("slug"))).filter(|v| js::truthy(Some(v)))
    };
    let exato = if usa_metas { mercado(d.get("eventSlug")).or_else(|| mercado(d.get("slug"))) } else { None };
    pnl_timestamp(d, exato.as_ref().and_then(|m| m.get("closedTime")))
}

/// `getStake(d)`: stake em $ (as fechadas trazem cotas em `totalBought`).
pub fn get_stake(d: &Value) -> R<ValorJs> {
    if d.is_null() {
        return Err(Excecao);
    }
    let (inicial, cotas, preco, tamanho) = (d.get("initialValue"), d.get("totalBought"), d.get("avgPrice"), d.get("size"));
    if nao_nulo(inicial) && js::numero(inicial) > 0.0 {
        return Ok(ValorJs::Bruto(inicial.cloned().unwrap_or(Value::Null)));
    }
    if nao_nulo(cotas) && nao_nulo(preco) && js::numero(preco) > 0.0 {
        return Ok(ValorJs::Numero(js::numero(cotas) * js::numero(preco)));
    }
    if nao_nulo(tamanho) && nao_nulo(preco) {
        return Ok(ValorJs::Numero(js::numero(tamanho) * js::numero(preco)));
    }
    Ok(ValorJs::Numero(0.0))
}

/// `getOddNum(d)`: `1 / avgPrice` (o `p <= 0` do JS é falso para `NaN`).
pub fn get_odd_num(d: &Value) -> R<f64> {
    if d.is_null() {
        return Err(Excecao);
    }
    let p = d.get("avgPrice");
    if !nao_nulo(p) || js::numero(p) <= 0.0 {
        return Ok(0.0);
    }
    Ok(1.0 / js::numero(p))
}

/// `JSON.stringify(n)`.
fn numero_em_json(n: &serde_json::Number) -> String {
    let f = n.as_f64().unwrap_or_else(|| n.to_string().parse().unwrap_or(f64::NAN));
    js::numero_texto(f).unwrap_or_else(|| "null".into())
}

/// `JSON.stringify(s)` de um texto, unidade UTF-16 por unidade (as metades de
/// um par substituto soltas viram `\udXXX`).
fn texto_json(unidades: &[u16], saida: &mut String) {
    saida.push('"');
    for c in char::decode_utf16(unidades.iter().copied()) {
        match c {
            Ok('"') => saida.push_str("\\\""),
            Ok('\\') => saida.push_str("\\\\"),
            Ok('\u{08}') => saida.push_str("\\b"),
            Ok('\u{0C}') => saida.push_str("\\f"),
            Ok('\n') => saida.push_str("\\n"),
            Ok('\r') => saida.push_str("\\r"),
            Ok('\t') => saida.push_str("\\t"),
            Ok(c) if c < ' ' => saida.push_str(&format!("\\u{:04x}", u32::from(c))),
            Ok(c) => saida.push(c),
            Err(e) => saida.push_str(&format!("\\u{:04x}", e.unpaired_surrogate())),
        }
    }
    saida.push('"');
}

/// `serializarLedgerEstavel(valor)`: JSON com as chaves em ordem (a do
/// `sort()` do JS, por unidades UTF-16).
fn serializar(v: &Value, saida: &mut String) {
    match v {
        Value::Null => saida.push_str("null"),
        Value::Bool(b) => saida.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => saida.push_str(&numero_em_json(n)),
        Value::String(s) => texto_json(&s.encode_utf16().collect::<Vec<_>>(), saida),
        Value::Array(itens) => {
            saida.push('[');
            for (i, item) in itens.iter().enumerate() {
                if i > 0 {
                    saida.push(',');
                }
                serializar(item, saida);
            }
            saida.push(']');
        }
        Value::Object(m) => serializar_objeto(m.iter().map(|(k, v)| (k.encode_utf16().collect(), Campo::Valor(v))).collect(), saida),
    }
}

/// Valor de um campo de `{...row}`: o do objeto, ou uma unidade UTF-16 de um texto espalhado.
enum Campo<'a> {
    Valor(&'a Value),
    Unidade(u16),
}

fn serializar_objeto(mut campos: Vec<(Vec<u16>, Campo<'_>)>, saida: &mut String) {
    campos.sort_by(|a, b| a.0.cmp(&b.0));
    saida.push('{');
    for (i, (k, v)) in campos.iter().enumerate() {
        if i > 0 {
            saida.push(',');
        }
        texto_json(k, saida);
        saida.push(':');
        match v {
            Campo::Valor(v) => serializar(v, saida),
            Campo::Unidade(u) => texto_json(&[*u], saida),
        }
    }
    saida.push('}');
}

fn base36(mut n: u32) -> String {
    let mut digitos = Vec::new();
    loop {
        digitos.push(char::from_digit(n % 36, 36).expect("dígito"));
        n /= 36;
        if n == 0 {
            break;
        }
    }
    digitos.iter().rev().collect()
}

/// `assinaturaFonteLedger(row)`: resumo da linha como veio da fonte (sem a
/// posição `_order`), para saber se ela mudou sem trazer o JSON do SQLite.
pub fn assinatura_fonte_ledger(linha: &Value) -> String {
    // `const { _order, ...dados } = row || {}`: o resto de um objeto, de uma
    // lista (chaves "0", "1"...) ou de um texto (uma chave por unidade UTF-16).
    let indice = |i: usize| i.to_string().encode_utf16().collect::<Vec<u16>>();
    let campos: Vec<(Vec<u16>, Campo<'_>)> = match linha {
        Value::Object(m) => m.iter().filter(|(k, _)| *k != "_order").map(|(k, v)| (k.encode_utf16().collect(), Campo::Valor(v))).collect(),
        Value::Array(itens) => itens.iter().enumerate().map(|(i, v)| (indice(i), Campo::Valor(v))).collect(),
        Value::String(s) => s.encode_utf16().enumerate().map(|(i, u)| (indice(i), Campo::Unidade(u))).collect(),
        _ => Vec::new(),
    };
    let mut texto = String::new();
    serializar_objeto(campos, &mut texto);
    let (mut a, mut b, mut tamanho) = (2_166_136_261u32, 0x9e37_79b9u32, 0usize);
    for c in texto.encode_utf16() {
        a = (a ^ u32::from(c)).wrapping_mul(16_777_619);
        b = (b ^ u32::from(c)).wrapping_mul(2_246_822_519);
        tamanho += 1;
    }
    format!("{tamanho}:{}:{}", base36(a), base36(b))
}

regex_exata!(VENCE_SIMPLES, r"(?i)^.+?\s+wins?\??$");

/// `titulo !== d.title` para o título que `tituloExibicao(d)` devolveu: um
/// objeto só é o mesmo quando veio do próprio `d.title`.
fn titulo_diferente(d: &Value, titulo: &Value) -> bool {
    let original = d.get("title");
    match titulo {
        Value::Object(_) | Value::Array(_) => verdadeiro(d.get("_titleDisplay")).is_some_and(|t| !titles::kalshi_title_missing_matchup(d, Some(&clean_v(Some(t))))),
        Value::Number(n) => !original.and_then(Value::as_f64).is_some_and(|o| Some(o) == n.as_f64()),
        outro => original != Some(outro),
    }
}

/// `versaoTituloConfirmado(d, titulo)`: a versão do título confirmado da
/// Kalshi, ou vazio. `Err` quando o JS lançaria exceção.
pub fn versao_titulo_confirmado(d: &Value, titulo: &Value) -> R<&'static str> {
    if titles::plataforma(d, "") != "kalshi" || titles::kalshi_title_missing_matchup(d, Some(&clean_v(Some(titulo)))) {
        return Ok("");
    }
    if !titles::kalshi_special_title_linha(d).is_empty() || !titles::inning_winner_title_linha(d).map_err(|()| Excecao)?.is_empty() {
        return Ok(VERSAO_INDICE_TITULO);
    }
    if d.get("_titleDisplayVersion").and_then(Value::as_str) == Some(VERSAO_INDICE_TITULO) {
        return Ok(VERSAO_INDICE_TITULO);
    }
    let Some(meta) = verdadeiro(d.get("_kalshiTitleMeta")) else { return Ok("") };
    let mercado = meta.get("market");
    let lado = js::texto_ou_vazio(d.get("outcome")).to_lowercase();
    let detalhe = titles::kalshi_detail_js(mercado.unwrap_or(&Value::Null), &lado, &Escopo::de_valor(meta.get("scope")), false);
    let vence = VENCE_SIMPLES.is_match(&js::texto_ou_vazio(mercado.and_then(|m| m.get("title"))));
    Ok(if titulo_diferente(d, titulo) || !detalhe.is_empty() || vence { VERSAO_INDICE_TITULO } else { "" })
}

/// `chaveLedgerFechada(d)`: a identidade estável da aposta no ledger.
pub fn chave_ledger_fechada(d: &Value) -> R<String> {
    let plataforma = match verdadeiro(d.get("_platform")) {
        None => "polymarket".to_string(),
        Some(Value::String(s)) => s.to_lowercase(),
        Some(_) => return Err(Excecao),
    };
    let texto = |v: Option<&Value>| js::texto_ou_vazio(v);
    Ok(match plataforma.as_str() {
        "predict" | "sxbet" => format!("{plataforma}|{}", texto(ou(d.get("slug"), d.get("id")))),
        // O ticker é a identidade do mercado; versões antigas mudavam o conditionId.
        "kalshi" => format!("kalshi|{}", js::trim_js(&texto(ou(ou(d.get("slug"), d.get("ticker")), d.get("market_ticker"))))),
        _ => {
            let resultado = [d.get("outcomeIndex"), d.get("outcome")].into_iter().flatten().find(|v| !v.is_null());
            format!("polymarket|{}|{}|{}", texto(d.get("conditionId")), texto(d.get("asset")), resultado.map_or(String::new(), |v| js::texto(Some(v))))
        }
    })
}

/// `ymdHistoricoDb(data)`: `AAAA-MM-DD` em São Paulo, como o `Intl.DateTimeFormat("en-CA")`.
fn ymd_historico(ms: Option<f64>) -> String {
    let Some(t) = ms.filter(|m| m.is_finite()).and_then(|m| Utc.timestamp_millis_opt(m.trunc() as i64).single()) else { return String::new() };
    let local = t.with_timezone(&Sao_Paulo);
    let ano = if local.year() >= 1 { local.year() } else { 1 - local.year() };
    format!("{ano}-{:02}-{:02}", local.month(), local.day())
}

/// `x || 0` de um número.
fn ou_zero(n: f64) -> f64 {
    if n.is_nan() || n == 0.0 { 0.0 } else { n }
}

/// `indiceHistoricoFechado(d)`: a linha de `closed_bets_index` de uma aposta
/// fechada, com categoria, título e datas. `Err` quando o JS lançaria exceção.
pub fn indice_historico_fechado(d: &Value, metas: Option<&Value>) -> R<Value> {
    let chave = chave_ledger_fechada(d)?;
    let (plataforma, aposta) = chave.split_once('|').map_or(("polymarket".to_string(), chave.clone()), |(p, a)| (p.to_string(), a.to_string()));
    if d.is_null() {
        return Err(Excecao);
    }
    let inicio_estrito = gamma::parse_api_date(d.get("_startDate"));
    let casa = d.get("_platform").and_then(Value::as_str);
    let aquisicao = if matches!(casa, Some("predict" | "sxbet")) { timestamp_ms(d.get("timestamp")).map(f64::trunc) } else { None };
    let tabela = if casa == Some("predict") { aquisicao } else { data_inicio_aposta(d)?.or(aquisicao) };
    let pnl_ts = get_ts_resultado_pnl(d, metas)?;
    let titulo = exibicao::titulo_exibicao(d)?;
    let zero = json!(0);
    let pnl = ou_zero(js::numero(ou(d.get("realizedPnl"), Some(&zero))));
    let stake = ou_zero(get_stake(d)?.numero());
    let odd = ou_zero(get_odd_num(d)?);
    let categoria = categorias::classificar(d)?;
    let assinatura = verdadeiro(d.get("_ledgerSourceSignature")).cloned().unwrap_or_else(|| Value::String(assinatura_fonte_ledger(d)));
    // `titulo.toLocaleLowerCase("pt-BR")`: um título que não é texto lança exceção.
    let Value::String(texto_titulo) = &titulo else { return Err(Excecao) };
    let busca = texto_titulo.to_lowercase();
    let versao = versao_titulo_confirmado(d, &titulo)?;
    Ok(json!({
        "platform": plataforma,
        "betKey": aposta,
        "category": categoria,
        "startYmd": ymd_historico(inicio_estrito),
        "tableYmd": ymd_historico(tabela),
        "tableTs": js::numero_json(tabela.unwrap_or(0.0)),
        "pnlTs": js::numero_json(pnl_ts),
        "pnlYmd": ymd_historico(Some(pnl_ts).filter(|t| *t != 0.0)),
        "pnlValue": js::numero_json(pnl),
        "stakeValue": js::numero_json(stake),
        "oddValue": js::numero_json(odd),
        "won": i32::from(pnl > 0.0),
        "sourceSignature": assinatura,
        "metricsReady": 1,
        "titleSearch": busca,
        "titleDisplay": titulo,
        "titleVersion": versao,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Um número no formato dos oráculos: `{"n": 1.5}`, ou `{"n": "NaN"}`,
    /// `{"n": "Infinity"}`, `{"n": "-0"}`.
    fn numero_do_oraculo(n: f64) -> Value {
        if n.is_nan() {
            json!({ "n": "NaN" })
        } else if n.is_infinite() {
            json!({ "n": if n > 0.0 { "Infinity" } else { "-Infinity" } })
        } else if n == 0.0 && n.is_sign_negative() {
            json!({ "n": "-0" })
        } else {
            json!({ "n": js::numero_json(n) })
        }
    }

    #[test]
    fn exemplos() {
        assert_eq!(get_stake(&json!({ "initialValue": "5" })), Ok(ValorJs::Bruto(json!("5"))));
        assert_eq!(get_stake(&json!({ "totalBought": 10, "avgPrice": "0.5" })).map(|v| v.numero()), Ok(5.0));
        assert!(get_odd_num(&json!({ "avgPrice": "x" })).unwrap().is_nan());
        assert_eq!(get_odd_num(&json!({ "avgPrice": 0 })), Ok(0.0));
        assert_eq!(data_inicio_aposta(&json!({ "_platform": 5 })), Err(Excecao));
        let ticker = data_inicio_aposta(&json!({ "_platform": "kalshi", "ticker": "KXMLBTEAMTOTAL-26AUG081805TORPHI" })).unwrap();
        assert_eq!(ticker.and_then(js::iso).as_deref(), Some("2026-08-08T22:05:00.000Z"));
        // A posição na lista não entra na assinatura.
        assert_eq!(assinatura_fonte_ledger(&json!({ "b": 1, "a": [1.50, "x"], "_order": 3 })), assinatura_fonte_ledger(&json!({ "a": [1.5, "x"], "b": 1 })));
        assert_eq!(assinatura_fonte_ledger(&json!(null)), assinatura_fonte_ledger(&json!({})));
    }

    #[test]
    fn datas_do_indice_como_o_icu() {
        assert_eq!(ymd_historico(Some(0.0)), "1969-12-31");
        assert_eq!(ymd_historico(Some(-58_998_369_600_000.0)), "100-06-01");
        assert_eq!(ymd_historico(Some(-77_932_584_000_000.0)), "501-06-01");
        assert_eq!(ymd_historico(Some(1_792_551_599_999.0)), "2026-10-20");
        assert_eq!(ymd_historico(None), "");
    }

    /// Oráculo contra o `indiceHistoricoFechado()` real: os casos e as
    /// respostas do JS vêm de `ferramentas/validacao/oraculo_indice.mjs indice`
    /// (`BETTY_ORACULO_INDICE=<arquivo .jsonl>`). Compara o JSON escrito.
    #[test]
    #[ignore]
    fn oraculo_indice() {
        let caminho = std::env::var("BETTY_ORACULO_INDICE").expect("defina BETTY_ORACULO_INDICE");
        let texto = std::fs::read_to_string(caminho).unwrap();
        let (mut total, mut diferentes, mut exemplos) = (0usize, 0usize, Vec::new());
        for linha in texto.lines().filter(|l| !l.trim().is_empty()) {
            let caso: Value = serde_json::from_str(linha).unwrap();
            let esperado = if caso.get("erro").is_some() { Err(Excecao) } else { Ok(js::para_texto(&caso["saida"])) };
            let obtido = indice_historico_fechado(&caso["entrada"]["d"], caso["entrada"].get("metas")).map(|v| js::para_texto(&v));
            total += 1;
            if obtido != esperado {
                diferentes += 1;
                if exemplos.len() < 20 {
                    exemplos.push(format!("{}:\n   JS   {:?}\n   Rust {:?}", caso["id"], esperado, obtido));
                }
            }
        }
        assert!(diferentes == 0, "{diferentes} de {total} casos diferentes; os primeiros:\n{}", exemplos.join("\n"));
        eprintln!("{total} casos iguais ao JS");
    }

    /// Oráculo contra os auxiliares reais de `indiceHistoricoFechado()`: os
    /// casos e as respostas do JS vêm de `ferramentas/validacao/oraculo_indice.mjs`
    /// (`BETTY_ORACULO_AJUDANTES=<arquivo .jsonl>`).
    #[test]
    #[ignore]
    fn oraculo_ajudantes_indice() {
        let caminho = std::env::var("BETTY_ORACULO_AJUDANTES").expect("defina BETTY_ORACULO_AJUDANTES");
        let texto = std::fs::read_to_string(caminho).unwrap();
        let (mut total, mut diferentes, mut exemplos) = (0usize, 0usize, Vec::new());
        let mut por_campo = std::collections::BTreeMap::<String, usize>::new();
        let resposta = |r: R<Value>| r.unwrap_or_else(|Excecao| json!({ "erro": true }));
        for linha in texto.lines().filter(|l| !l.trim().is_empty()) {
            let caso: Value = serde_json::from_str(linha).unwrap();
            let (d, metas) = (&caso["entrada"]["d"], caso["entrada"].get("metas"));
            let obtido = json!({
                "dataInicio": resposta(data_inicio_aposta(d).map(|t| t.map_or(json!({ "v": null }), |t| json!({ "data": js::numero_json(t) })))),
                "tsPnl": resposta(get_ts_resultado_pnl(d, metas).map(numero_do_oraculo)),
                "stake": resposta(get_stake(d).map(|v| match v {
                    ValorJs::Bruto(Value::Number(n)) => numero_do_oraculo(n.as_f64().unwrap_or(f64::NAN)),
                    ValorJs::Bruto(v) => json!({ "v": v }),
                    ValorJs::Numero(n) => numero_do_oraculo(n),
                })),
                "odd": resposta(get_odd_num(d).map(numero_do_oraculo)),
                "assinatura": json!({ "v": assinatura_fonte_ledger(d) }),
                "versao": resposta(crate::exibicao::titulo_exibicao(d).and_then(|t| versao_titulo_confirmado(d, &t)).map(|v| json!({ "v": v }))),
            });
            let esperado = &caso["saida"];
            total += 1;
            let mut igual = true;
            for (k, v) in obtido.as_object().unwrap() {
                if js::para_texto(v) != js::para_texto(&esperado[k]) {
                    igual = false;
                    *por_campo.entry(k.clone()).or_default() += 1;
                    if exemplos.len() < 25 {
                        exemplos.push(format!("{} {k}: JS {} x Rust {}", caso["id"], esperado[k], v));
                    }
                }
            }
            if !igual {
                diferentes += 1;
            }
        }
        assert!(diferentes == 0, "{diferentes} de {total} casos diferentes ({por_campo:?}); os primeiros:\n{}", exemplos.join("\n"));
        eprintln!("{total} casos iguais ao JS (6 campos cada)");
    }
}
