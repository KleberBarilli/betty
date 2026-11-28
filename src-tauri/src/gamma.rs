//! Metadados de eventos da Gamma (Polymarket) e a data de início de uma
//! aposta, iguais a `metaDatasGamma()`, `extrairMetaEsporteEvento()` (com
//! `tokenIdsDoMercado()`) e `resolverDataInicio()` (com `dataDoSlug()`,
//! `metaDoMercado()`, `inicioAdministrativo()`, `datasProximas()`,
//! `eMercadoDeCampeao()`, `dataMs()` e `parseApiDate()`) do `dashboard.js`.
//!
//! Parte da sincronização no Rust (`docs/desempenho.md`): ainda não é usada
//! pelas rotas, e o oráculo (`tests::oraculo_gamma`) compara com o JS real.
//!
//! - `new Date(s)` segue o V8 nas datas `AAAA-MM-DD[THH:MM[:SS[.fff]]][fuso]`
//!   (o caminho ES5, estrito depois do `T`, com dia até 31 rolando para o mês
//!   seguinte e `24:00` só zerado) e no caminho antigo com espaço
//!   (`AAAA-M-D H:MM:SS ±H`, `GMT`, `UTC`), mais a RFC 2822. Outros formatos
//!   antigos do V8 (nome do mês, barras, números soltos: o V8 lê "7" como
//!   julho de 2001 e "50" como 1950) ficam `None`, e a hora local que não
//!   existe (salto do horário de verão) também.
//! - Chaves herdadas do protótipo (`constructor`, `__proto__`...) não existem
//!   nos mapas daqui.
//! - `JSON.parse` dos `clobTokenIds` é o do `serde_json`: um texto com metade
//!   de um par substituto ou mais de 128 níveis falha aqui e não no JS.

use chrono::{Duration, NaiveDate, TimeZone};
use serde_json::{Map, Value, json};

use crate::categorias::Excecao;
use crate::titles::{self, ou};
use crate::{js, regex_exata};

const CAMPOS_DATA: [&str; 9] =
    ["startTime", "startDate", "gameStartTime", "eventDate", "endDate", "closedTime", "umaEndDate", "creationDate", "acceptingOrdersTimestamp"];

fn verdadeiro(v: Option<&Value>) -> Option<&Value> {
    v.filter(|v| js::truthy(Some(v)))
}

/// `x || null`.
fn ou_nulo(v: Option<&Value>) -> Value {
    verdadeiro(v).cloned().unwrap_or(Value::Null)
}

/// `x ?? y`.
fn nao_nulo<'a>(x: Option<&'a Value>, y: Option<&'a Value>) -> Option<&'a Value> {
    x.filter(|v| !v.is_null()).or(y)
}

/// `campos(obj)` de `metaDatasGamma()`.
fn campos(obj: Option<&Value>) -> Value {
    let campo = |k: &str| obj.and_then(|o| o.get(k));
    let mut m = Map::new();
    for k in CAMPOS_DATA {
        m.insert(k.into(), ou_nulo(campo(k)));
    }
    m.insert("description".into(), verdadeiro(campo("description")).cloned().unwrap_or_else(|| json!("")));
    m.insert("title".into(), verdadeiro(ou(campo("title"), campo("question"))).cloned().unwrap_or_else(|| json!("")));
    Value::Object(m)
}

/// Índice inteiro de objeto do JS (`"0"`, `"12"`...), que vem antes das outras chaves.
fn indice_de_objeto(chave: &str) -> Option<u64> {
    let canonico = chave == "0" || (!chave.is_empty() && !chave.starts_with('0') && chave.bytes().all(|b| b.is_ascii_digit()));
    chave.parse::<u64>().ok().filter(|n| canonico && *n < 4_294_967_295)
}

/// Objeto com as chaves na ordem do JS: índices inteiros primeiro, em ordem
/// numérica, e o resto na ordem de inserção.
fn objeto_na_ordem_js(pares: Vec<(String, Value)>) -> Map<String, Value> {
    let (mut indices, resto): (Vec<_>, Vec<_>) = pares.into_iter().partition(|(k, _)| indice_de_objeto(k).is_some());
    indices.sort_by_key(|(k, _)| indice_de_objeto(k));
    indices.into_iter().chain(resto).collect()
}

/// `metaDatasGamma(event)`. `Err` quando `event.markets` não é iterável.
pub fn meta_datas_gamma(evento: Option<&Value>) -> Result<Value, Excecao> {
    let mut pares: Vec<(String, Value)> = Vec::new();
    for mercado in titles::espalhar(evento.and_then(|e| e.get("markets"))).map_err(|()| Excecao)? {
        let Some(slug) = verdadeiro(mercado.get("slug")) else { continue };
        let chave = js::texto(Some(slug));
        // `markets["__proto__"] = {...}` troca o protótipo: não vira chave.
        if chave == "__proto__" {
            continue;
        }
        let valor = campos(Some(&mercado));
        match pares.iter_mut().find(|(k, _)| *k == chave) {
            Some(par) => par.1 = valor,
            None => pares.push((chave, valor)),
        }
    }
    Ok(json!({ "event": campos(evento), "markets": Value::Object(objeto_na_ordem_js(pares)) }))
}

/// `add(v)` de `extrairMetaEsporteEvento()`/`tokenIdsDoMercado()`.
fn juntar(textos: &mut Vec<String>, v: Option<&Value>) {
    if let Some(v) = v.filter(|v| !v.is_null()) {
        let texto = js::texto(Some(v));
        let t = js::trim_js(&texto);
        if !t.is_empty() {
            textos.push(t.to_string());
        }
    }
}

/// `[...new Set(itens)]`.
fn sem_repetir(itens: &[String]) -> Vec<String> {
    let mut vistos = std::collections::HashSet::new();
    itens.iter().filter(|t| vistos.insert(t.as_str())).cloned().collect()
}

/// `Array.isArray(v) ? v : []`.
fn lista(v: Option<&Value>) -> &[Value] {
    v.and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

/// `tokenIdsDoMercado(m)`: os token IDs que identificam o YES/NO exato.
pub fn token_ids_do_mercado(m: &Value) -> Vec<String> {
    let mut saida = Vec::new();
    for campo in [m.get("clobTokenIds"), m.get("tokenIds")] {
        match campo {
            Some(Value::Array(a)) => a.iter().for_each(|v| juntar(&mut saida, Some(v))),
            Some(Value::String(s)) if !js::trim_js(s).is_empty() => match serde_json::from_str::<Value>(s) {
                Ok(Value::Array(a)) => a.iter().for_each(|v| juntar(&mut saida, Some(v))),
                Ok(_) => juntar(&mut saida, campo),
                Err(_) => s.split(',').for_each(|p| juntar(&mut saida, Some(&Value::String(p.into())))),
            },
            _ => {}
        }
    }
    for t in lista(m.get("tokens")) {
        juntar(&mut saida, nao_nulo(nao_nulo(t.get("token_id"), t.get("tokenId")), t.get("id")));
    }
    sem_repetir(&saida)
}

/// `extrairMetaEsporteEvento(event)`.
pub fn extrair_meta_esporte_evento(evento: Option<&Value>) -> Value {
    let Some(e) = verdadeiro(evento) else { return Value::Null };
    let mut textos = Vec::new();
    for k in ["category", "subcategory", "title", "subtitle", "slug", "ticker", "seriesSlug"] {
        juntar(&mut textos, e.get(k));
    }
    let rotulos = |textos: &mut Vec<String>, itens: &[Value], chaves: &[&str]| {
        for item in itens {
            for k in chaves {
                juntar(textos, item.get(*k));
            }
        }
    };
    rotulos(&mut textos, lista(e.get("tags")), &["label", "slug"]);
    rotulos(&mut textos, lista(e.get("categories")), &["label", "slug", "parentCategory"]);
    for serie in lista(e.get("series")) {
        for k in ["title", "subtitle", "slug", "ticker", "description"] {
            juntar(&mut textos, serie.get(k));
        }
        rotulos(&mut textos, lista(serie.get("tags")), &["label", "slug"]);
        rotulos(&mut textos, lista(serie.get("categories")), &["label", "slug"]);
    }
    let textos_evento = sem_repetir(&textos);
    let mut tipos = Vec::new();
    let mut mercados = Vec::new();
    for m in lista(e.get("markets")) {
        for k in ["category", "sportsMarketType", "slug", "question", "groupItemTitle"] {
            juntar(&mut textos, m.get(k));
        }
        if let Some(t) = verdadeiro(m.get("sportsMarketType")) {
            tipos.push(js::texto(Some(t)));
        }
        rotulos(&mut textos, lista(m.get("tags")), &["label", "slug"]);
        rotulos(&mut textos, lista(m.get("categories")), &["label", "slug"]);
        mercados.push(json!({
            "conditionId": ou_nulo(ou(m.get("conditionId"), m.get("condition_id"))),
            "slug": ou_nulo(m.get("slug")),
            "question": ou_nulo(m.get("question")),
            "groupItemTitle": ou_nulo(m.get("groupItemTitle")),
            "description": ou_nulo(m.get("description")),
            "sportsMarketType": ou_nulo(m.get("sportsMarketType")),
            "line": nao_nulo(nao_nulo(m.get("line"), m.get("groupItemThreshold")), None).cloned().unwrap_or(Value::Null),
            "category": ou_nulo(m.get("category")),
            "tokenIds": token_ids_do_mercado(m),
        }));
    }
    json!({
        "texts": sem_repetir(&textos),
        "eventTexts": textos_evento,
        "sportsMarketTypes": sem_repetir(&tipos),
        "markets": mercados,
        "eventId": ou_nulo(e.get("id")),
        "seriesSlug": ou_nulo(e.get("seriesSlug")),
    })
}

/// Fuso de uma data lida.
enum Fuso {
    Utc,
    Minutos(i64),
    Local,
}

/// `MakeDate` com o dia rolando para o mês seguinte e `24:00` como o dia seguinte.
#[allow(clippy::too_many_arguments)]
fn compor(ano: i32, mes: u32, dia: u32, hora: u32, minuto: u32, segundo: u32, ms: u32, fuso: Fuso) -> Option<f64> {
    let valida = (1..=12).contains(&mes)
        && (1..=31).contains(&dia)
        && minuto <= 59
        && segundo <= 59
        && (hora <= 23 || (hora == 24 && minuto == 0 && segundo == 0 && ms == 0));
    if !valida {
        return None;
    }
    let data = NaiveDate::from_ymd_opt(ano, mes, 1)?.checked_add_signed(Duration::days(i64::from(dia) - 1))?;
    let hora_do_dia = data.and_hms_milli_opt(hora % 24, minuto, segundo, ms)? + Duration::days(i64::from(hora / 24));
    let t = match fuso {
        Fuso::Utc => hora_do_dia.and_utc().timestamp_millis(),
        Fuso::Minutos(m) => hora_do_dia.and_utc().timestamp_millis() - m * 60_000,
        Fuso::Local => chrono::Local.from_local_datetime(&hora_do_dia).earliest()?.timestamp_millis(),
    };
    ((t as f64).abs() <= 8.64e15).then_some(t as f64)
}

/// Milissegundos de uma fração (`.5` = 500, `.123456` = 123).
fn fracao_ms(f: Option<regex::Match<'_>>) -> u32 {
    f.map_or(0, |m| m.as_str().chars().chain(std::iter::repeat('0')).take(3).collect::<String>().parse().unwrap_or(0))
}

regex_exata!(
    DATA_ES5,
    r"^(?:([+-]\d{6})|(\d{4}))(?:-(\d{2})(?:-(\d{2}))?)?(?:[Tt](\d{2}):(\d{2})(?::(\d{2})(?:\.(\d+))?)?(?:([Zz])|([+-])(\d{2}):?(\d{2}))?)?$"
);
regex_exata!(DATA_COM_T, r"^(?:[+-]\d{6}|\d{4})(?:-\d{2}(?:-\d{2})?)?[Tt]");
regex_exata!(
    DATA_ANTIGA,
    r"^(\d{4})-(\d{1,2})-(\d{1,2})(?:\s+(\d{1,2}):(\d{2})(?::(\d{2})(?:\.(\d+))?)?)?\s*(?:([Zz]|UTC|GMT)|([+-])(\d{1,4})(?::(\d{2}))?)?$"
);

/// `new Date(s).getTime()` (`None` = `NaN`).
pub fn data_js(s: &str) -> Option<f64> {
    let n = |m: Option<regex::Match<'_>>, padrao: u32| m.map_or(Some(padrao), |m| m.as_str().parse::<u32>().ok());
    if let Some(c) = DATA_ES5.captures(s) {
        let ano: i32 = c.get(1).or(c.get(2))?.as_str().parse().ok()?;
        let tem_hora = c.get(5).is_some();
        let fuso = if c.get(9).is_some() {
            Fuso::Utc
        } else if let Some(sinal) = c.get(10) {
            let (h, m) = (n(c.get(11), 0)?, n(c.get(12), 0)?);
            if h > 23 || m > 59 {
                return None;
            }
            Fuso::Minutos(if sinal.as_str() == "-" { -1 } else { 1 } * i64::from(h * 60 + m))
        } else if tem_hora {
            Fuso::Local
        } else {
            Fuso::Utc
        };
        return compor(ano, n(c.get(3), 1)?, n(c.get(4), 1)?, n(c.get(5), 0)?, n(c.get(6), 0)?, n(c.get(7), 0)?, fracao_ms(c.get(8)), fuso);
    }
    // Depois do `T`, o V8 não volta para o leitor antigo.
    if DATA_COM_T.is_match(s) {
        return None;
    }
    if let Some(c) = DATA_ANTIGA.captures(s) {
        let fuso = if c.get(8).is_some() {
            Fuso::Utc
        } else if let Some(sinal) = c.get(9) {
            let digitos = c.get(10)?.as_str();
            let v: i64 = digitos.parse().ok()?;
            let minutos = match (digitos.len(), c.get(11)) {
                (1 | 2, None) => v * 60,
                (1 | 2, Some(m)) => v * 60 + m.as_str().parse::<i64>().ok()?,
                (_, None) => v / 100 * 60 + v % 100,
                (_, Some(_)) => return None,
            };
            Fuso::Minutos(if sinal.as_str() == "-" { -minutos } else { minutos })
        } else {
            Fuso::Local
        };
        return compor(c[1].parse().ok()?, n(c.get(2), 1)?, n(c.get(3), 1)?, n(c.get(4), 0)?, n(c.get(5), 0)?, n(c.get(6), 0)?, fracao_ms(c.get(7)), fuso);
    }
    chrono::DateTime::parse_from_rfc2822(s).ok().map(|d| d.timestamp_millis() as f64)
}

regex_exata!(PAD_ISO, r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}");
regex_exata!(PAD_FUSO, r"[zZ]|[+-]\d{2}:?\d{2}$");

/// `parseApiDate(raw)` em milissegundos: ISO sem fuso é UTC, não hora local.
pub fn parse_api_date(raw: Option<&Value>) -> Option<f64> {
    let raw = verdadeiro(raw)?;
    let mut s = js::trim_js(&js::texto(Some(raw))).to_string();
    if PAD_ISO.is_match(&s) && !PAD_FUSO.is_match(&s) {
        s.push('Z');
    }
    data_js(&s)
}

/// `dataMs(raw)`.
pub fn data_ms(raw: Option<&Value>) -> f64 {
    parse_api_date(raw).unwrap_or(0.0)
}

/// `datasProximas(a, b)`: as duas datas válidas e a até 24 h uma da outra.
fn datas_proximas(a: Option<&Value>, b: Option<&Value>) -> bool {
    let (ta, tb) = (data_ms(a), data_ms(b));
    ta > 0.0 && tb > 0.0 && (ta - tb).abs() <= 86_400_000.0
}

regex_exata!(DATA_NO_SLUG, r"(20\d{2}-\d{2}-\d{2})");

/// `dataDoSlug(slug)`. `Err` quando o slug não é texto (`(5).match`).
fn data_do_slug(slug: Option<&Value>) -> Result<Option<String>, Excecao> {
    match slug {
        Some(Value::String(s)) => Ok(DATA_NO_SLUG.captures(s).map(|c| format!("{}T15:00:00Z", &c[1]))),
        Some(v) if js::truthy(Some(v)) => Err(Excecao),
        _ => Ok(None),
    }
}

/// `POLYMARKET_DATE_OVERRIDES`: datas esportivas confirmadas fora da Gamma.
fn data_confirmada(chave: &str) -> Option<&'static str> {
    match chave {
        "will-neymar-play-in-the-world-cup" => Some("2026-06-24T22:00:00Z"),
        "world-cup-nation-to-reach-quarterfinals|will-france-reach-the-quarterfinals-at-the-2026-fifa-world-cup-20260602145134499" => {
            Some("2026-07-04T21:00:00Z")
        }
        _ => None,
    }
}

/// `{ event, market }` de `resolverDataInicio()`.
struct MetaDoEvento {
    evento: Option<Value>,
    mercado: Option<Value>,
}

impl MetaDoEvento {
    fn evento(&self, k: &str) -> Option<&Value> {
        self.evento.as_ref().and_then(|e| e.get(k))
    }

    fn mercado(&self, k: &str) -> Option<&Value> {
        self.mercado.as_ref().and_then(|m| m.get(k))
    }
}

/// `inicioAdministrativo(meta, dataPadrao)`: o início é a data em que o
/// mercado foi aberto, não a do jogo.
fn inicio_administrativo(meta: Option<&MetaDoEvento>, padrao: &Value) -> bool {
    let Some(meta) = meta else { return false };
    let vazio = Value::Object(Map::new());
    let evento = verdadeiro(meta.evento.as_ref()).unwrap_or(&vazio);
    let mercado = verdadeiro(meta.mercado.as_ref()).unwrap_or(&vazio);
    let filtrar = |v: [Option<&Value>; 4]| -> Vec<Value> { v.into_iter().filter_map(verdadeiro).cloned().collect() };
    let aberturas = filtrar([mercado.get("creationDate"), evento.get("creationDate"), mercado.get("acceptingOrdersTimestamp"), evento.get("acceptingOrdersTimestamp")]);
    let inicios = filtrar([mercado.get("startDate"), evento.get("startTime"), evento.get("startDate"), None]);
    inicios.iter().any(|i| datas_proximas(Some(i), Some(padrao)))
        && aberturas.iter().any(|a| inicios.iter().any(|i| datas_proximas(Some(i), Some(a))))
}

regex_exata!(CAMPEAO_NAO, r"\b(?:reach|qualif(?:y|ies)|advance|play in)\b");
regex_exata!(CAMPEAO_SIM, r"\b(?:winner|champion|championship)\b");

/// `eMercadoDeCampeao(d, meta, inicio, fim)`: campeão de um torneio de pelo menos 7 dias.
fn e_mercado_de_campeao(d: &Value, meta: &MetaDoEvento, inicio: &Value, fim: &Value) -> bool {
    let texto = [d.get("title"), meta.evento("title"), meta.evento("description"), meta.mercado("description")]
        .into_iter()
        .filter_map(verdadeiro)
        .map(|v| js::texto(Some(v)))
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    if CAMPEAO_NAO.is_match(&texto) || !CAMPEAO_SIM.is_match(&texto) {
        return false;
    }
    let (ti, tf) = (data_ms(Some(inicio)), data_ms(Some(fim)));
    ti > 0.0 && tf > ti && tf - ti >= 7.0 * 86_400_000.0
}

regex_exata!(COPA_VENCEDOR, r"(?i)^will\s+.+\s+win\s+the\s+2026\s+fifa\s+world\s+cup\?$");

fn data_resolvida(data: Value, fonte: &str, suspeita: bool) -> Value {
    json!({ "data": data, "fonte": fonte, "suspeita": suspeita })
}

/// `resolverDataInicio(d, startBySlug, metasBySlug)`: `{data, fonte, suspeita}`.
/// `Err` quando o JS lançaria exceção.
pub fn resolver_data_inicio(d: &Value, inicios: &Value, metas: Option<&Value>) -> Result<Value, Excecao> {
    let evento_slug = d.get("eventSlug");
    let do_mapa = if js::truthy(evento_slug) { js::propriedade(inicios, &js::texto(evento_slug)) } else { None };
    let padrao = match do_mapa.filter(|v| js::truthy(Some(v))) {
        Some(v) => v,
        None => match data_do_slug(evento_slug)? {
            Some(s) => Value::String(s),
            None => match data_do_slug(d.get("slug"))? {
                Some(s) => Value::String(s),
                None => ou_nulo(d.get("_startDate")),
            },
        },
    };
    let chave = format!("{}|{}", js::texto_ou_vazio(evento_slug), js::texto_ou_vazio(d.get("slug")));
    if let Some(data) = data_confirmada(&chave).or_else(|| data_confirmada(&js::texto_ou_vazio(evento_slug))) {
        return Ok(data_resolvida(json!(data), "override-esportivo-confirmado", true));
    }
    // Campeão do torneio: a Gamma informa endDate como o fim esportivo da final.
    if evento_slug.and_then(Value::as_str) == Some("world-cup-winner") && COPA_VENCEDOR.is_match(&js::texto_ou_vazio(d.get("title"))) {
        if let Some(fim) = verdadeiro(d.get("endDate")) {
            return Ok(data_resolvida(fim.clone(), "endDate-final-do-torneio", true));
        }
    }
    let meta = metas.and_then(|m| js::propriedade(m, &js::texto(evento_slug))).filter(|e| js::truthy(Some(e))).map(|e| MetaDoEvento {
        evento: e.get("event").cloned(),
        mercado: e.get("markets").filter(|m| !m.is_null()).and_then(|m| js::propriedade(m, &js::texto(d.get("slug")))).filter(|m| js::truthy(Some(m))),
    });
    let Some(meta) = meta.filter(|m| inicio_administrativo(Some(m), &padrao)) else {
        return Ok(data_resolvida(padrao, "startDate-evento", false));
    };
    let do_evento = [meta.mercado("gameStartTime"), meta.evento("gameStartTime"), meta.mercado("eventDate"), meta.evento("eventDate")]
        .into_iter()
        .find(|v| data_ms(*v) > 0.0)
        .flatten();
    if let Some(data) = do_evento {
        return Ok(data_resolvida(data.clone(), "campo-explicito-do-evento", true));
    }
    let fim = ou_nulo(ou(ou(meta.mercado("endDate"), meta.evento("endDate")), d.get("endDate")));
    if js::truthy(Some(&fim)) && e_mercado_de_campeao(d, &meta, &padrao, &fim) {
        return Ok(data_resolvida(fim, "endDate-final-do-torneio", true));
    }
    // closedTime/umaEndDate são administrativos: sem data esportiva, fica a atual.
    Ok(data_resolvida(padrao, "startDate-suspeito-sem-alternativa", true))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datas_como_o_v8() {
        let ms = |s: &str| data_js(s).map(|t| js::iso(t).unwrap());
        assert_eq!(ms("2026-10-20 23:00:00+00").as_deref(), Some("2026-10-20T23:00:00.000Z"));
        assert_eq!(ms("2026-02-31").as_deref(), Some("2026-03-03T00:00:00.000Z"));
        assert_eq!(ms("2026-10-20T24:00:00Z").as_deref(), Some("2026-10-21T00:00:00.000Z"));
        assert_eq!(ms("2026-10-20T23:00:00+0300").as_deref(), Some("2026-10-20T20:00:00.000Z"));
        assert_eq!(ms("2026-10-20 23:00:00+2400").as_deref(), Some("2026-10-19T23:00:00.000Z"));
        assert_eq!(ms("2026").as_deref(), Some("2026-01-01T00:00:00.000Z"));
        for invalida in ["2026-10-20T23:00:00+03", "2026-02-32", "2026-10-20T24:00:01Z", "2026-10-20T1:00:00Z", "1789059900", "2026-10-20\u{85}"] {
            assert_eq!(data_js(invalida), None, "{invalida}");
        }
    }

    #[test]
    fn chaves_na_ordem_do_js() {
        let e = json!({ "markets": [{ "slug": "b" }, { "slug": "10" }, { "slug": "2" }, { "slug": "__proto__" }, { "slug": "b", "title": "x" }] });
        let m = meta_datas_gamma(Some(&e)).unwrap();
        assert_eq!(m["markets"].as_object().unwrap().keys().collect::<Vec<_>>(), ["2", "10", "b"]);
        assert_eq!(m["markets"]["b"]["title"], "x");
        assert_eq!(meta_datas_gamma(Some(&json!({ "markets": 5 }))), Err(Excecao));
    }

    #[test]
    fn token_ids() {
        let m = json!({ "clobTokenIds": "[\"1\", 2, null, 12345678901234567890]", "tokenIds": "a, b,a", "tokens": [{ "token_id": null, "tokenId": "c" }, { "id": 3 }] });
        assert_eq!(token_ids_do_mercado(&m), ["1", "2", "12345678901234567000", "a", "b", "c", "3"]);
    }

    /// Oráculo contra o `metaDatasGamma()`, o `extrairMetaEsporteEvento()` e o
    /// `resolverDataInicio()` reais: os casos e as respostas do JS vêm de
    /// `ferramentas/validacao/oraculo_gamma.mjs` (`BETTY_ORACULO_GAMMA=<arquivo .jsonl>`).
    /// Compara o JSON escrito, com a ordem das chaves.
    #[test]
    #[ignore]
    fn oraculo_gamma() {
        let caminho = std::env::var("BETTY_ORACULO_GAMMA").expect("defina BETTY_ORACULO_GAMMA");
        let texto = std::fs::read_to_string(caminho).unwrap();
        let (mut total, mut diferentes, mut exemplos) = (0usize, 0usize, Vec::new());
        let mut por_funcao = std::collections::BTreeMap::<String, usize>::new();
        for linha in texto.lines().filter(|l| !l.trim().is_empty()) {
            let caso: Value = serde_json::from_str(linha).unwrap();
            let e = &caso["entrada"];
            let funcao = e["f"].as_str().unwrap();
            let esperado = if caso.get("erro").is_some() { Err(Excecao) } else { Ok(js::para_texto(&caso["saida"])) };
            let obtido = match funcao {
                "meta" => meta_datas_gamma(e.get("evento")),
                "esporte" => Ok(extrair_meta_esporte_evento(e.get("evento"))),
                "inicio" => resolver_data_inicio(&e["d"], &e["inicios"], e.get("metas")),
                outra => panic!("função {outra}"),
            }
            .map(|v| js::para_texto(&v));
            total += 1;
            *por_funcao.entry(funcao.to_string()).or_default() += 1;
            if obtido != esperado {
                diferentes += 1;
                if exemplos.len() < 25 {
                    exemplos.push(format!("{}: JS {:?}\n   Rust {:?}", caso["id"], esperado, obtido));
                }
            }
        }
        assert!(diferentes == 0, "{diferentes} de {total} casos diferentes; os primeiros:\n{}", exemplos.join("\n"));
        eprintln!("{total} casos iguais ao JS: {por_funcao:?}");
    }
}
