//! Regras de data de resultado, iguais a `public/audit-utils.js`:
//! `timestampMs`, `pnlTimestamp`, `resolvedLoss` e `protectClosedDate`.

use chrono::{Datelike, TimeZone, Utc};
use serde_json::{Map, Value};

use crate::js;

/// `timestampMs()`: segundos ou ms (ou texto de data) entre 2000 e 2100.
pub fn timestamp_ms(v: Option<&Value>) -> Option<f64> {
    let v = v?;
    match v {
        Value::Null => return None,
        Value::String(s) if s.is_empty() => return None,
        _ => {}
    }
    let n = js::number(Some(v));
    let ms = if n.is_finite() {
        if n.abs() < 1e12 { n * 1000.0 } else { n }
    } else {
        js::data_ms(&js::string(Some(v)))?
    };
    if !ms.is_finite() || ms.abs() > 8.64e15 {
        return None;
    }
    let ano = Utc.timestamp_millis_opt(ms.trunc() as i64).single()?.year();
    (2000..=2100).contains(&ano).then_some(ms)
}

/// `a || b || c` de valores brutos.
fn primeiro_truthy<'a>(d: &'a Value, chaves: &[&str]) -> Option<&'a Value> {
    chaves.iter().map(|k| d.get(*k)).find(|v| js::truthy(*v)).flatten().or_else(|| chaves.last().and_then(|k| d.get(*k)))
}

fn plataforma(d: &Value) -> String {
    let p = d.get("_platform");
    if js::truthy(p) { js::string(p).to_lowercase() } else { "polymarket".into() }
}

/// `resolvedLoss()`: perda resolvida da Polymarket ainda não resgatada.
pub fn resolved_loss(d: &Value) -> bool {
    plataforma(d) == "polymarket"
        && d.get("redeemable") == Some(&Value::Bool(true))
        && d.get("curPrice").is_some_and(|v| !v.is_null())
        && js::number(d.get("curPrice")) == 0.0
}

/// `pnlTimestamp()`: instante em que o resultado foi realizado (0 = desconhecido).
pub fn pnl_timestamp(d: &Value, resolucao_mercado: Option<&Value>) -> f64 {
    if let Some(c) = timestamp_ms(d.get("_confirmedClosedTs")).filter(|c| *c != 0.0) {
        return c;
    }
    if matches!(d.get("_platform").and_then(Value::as_str), Some("predict" | "sxbet")) {
        return timestamp_ms(d.get("_realizedAt")).unwrap_or(0.0);
    }
    let ou = |t: Option<f64>| t.filter(|v| *v != 0.0);
    if plataforma(d) == "kalshi" {
        return ou(timestamp_ms(primeiro_truthy(d, &["_settledTime", "settled_time", "settlement_time"])))
            .or_else(|| ou(timestamp_ms(primeiro_truthy(d, &["_saleTs", "_lastTradeTs"]))))
            .unwrap_or(0.0);
    }
    let resolvida = d.get("_resolvedTime").filter(|v| js::truthy(Some(v))).or(resolucao_mercado);
    let resolucao = ou(timestamp_ms(resolvida));
    // Uma perda resolvida não resgatada ainda carrega a data da compra.
    if resolved_loss(d) {
        return resolucao.unwrap_or(0.0);
    }
    let fechada = ou(timestamp_ms(primeiro_truthy(d, &["_saleTs", "_lastTradeTs", "timestamp"])));
    match (fechada, resolucao) {
        (Some(f), Some(r)) => f.min(r),
        (f, r) => f.or(r).unwrap_or(0.0),
    }
}

/// `protectClosedDate()`: a data confirmada de uma aposta só muda com nova
/// venda comprovada ou com a recuperação da data oficial de resolução.
pub fn protect_closed_date(antiga: Option<&Value>, nova: Value) -> Value {
    let mut linha: Map<String, Value> = match nova {
        Value::Object(o) => o,
        _ => Map::new(),
    };
    // A data é derivada pelo servidor: um campo vindo da API não a confirma.
    linha.shift_remove("_confirmedClosedTs");
    let linha_v = Value::Object(linha);
    let candidata = pnl_timestamp(&linha_v, None);
    let Value::Object(mut linha) = linha_v else { unreachable!("linha é objeto") };
    let vazio = Value::Object(Map::new());
    let antiga_v = antiga.unwrap_or(&vazio);
    let anterior = pnl_timestamp(antiga_v, None);
    let plataforma_linha = linha.get("_platform").filter(|v| js::truthy(Some(v))).or_else(|| antiga_v.get("_platform"));
    let kalshi = (if js::truthy(plataforma_linha) { js::string(plataforma_linha) } else { "polymarket".into() }).to_lowercase() == "kalshi";
    let evidencia = linha.get("_saleEvidence");
    let venda_ts = timestamp_ms(evidencia.and_then(|e| e.get("timestamp")));
    let hash = evidencia.and_then(|e| e.get("transactionHash"));
    let texto_ou_vazio = |v: Option<&Value>| if js::truthy(v) { js::string(v) } else { String::new() };
    let nova_venda = !kalshi
        && js::truthy(hash)
        && texto_ou_vazio(evidencia.and_then(|e| e.get("asset"))) == texto_ou_vazio(linha.get("asset"))
        && venda_ts.is_some_and(|v| v > anterior)
        && hash != antiga_v.get("_saleEvidence").and_then(|e| e.get("transactionHash"));
    let resolucao_recuperada = !kalshi
        && timestamp_ms(antiga_v.get("_resolvedTime")).is_none_or(|v| v == 0.0)
        && timestamp_ms(linha.get("_resolvedTime")).is_some_and(|v| v != 0.0)
        && candidata > 0.0
        && candidata < anterior;
    let mut confirmada = if anterior != 0.0 { anterior } else { candidata };
    if nova_venda {
        let venda = venda_ts.unwrap_or(0.0);
        confirmada = match timestamp_ms(linha.get("_resolvedTime")).filter(|v| *v != 0.0) {
            Some(r) => venda.min(r),
            None => venda,
        };
    } else if resolucao_recuperada {
        confirmada = candidata;
    }
    if kalshi && timestamp_ms(antiga_v.get("_settledTime")).is_some_and(|v| v != 0.0) {
        linha.insert("_settledTime".into(), antiga_v["_settledTime"].clone());
    }
    if confirmada > 0.0 {
        linha.insert("_confirmedClosedTs".into(), js::numero_json(confirmada));
    }
    Value::Object(linha)
}
