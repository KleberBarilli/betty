//! Regras puras da Kalshi que ficam no `server.js`: título da aposta, data
//! oficial do jogo, modalidade esportiva e as chaves/assinaturas usadas no
//! cache de liquidações. Nada aqui faz rede ou acessa o banco.

use serde_json::{Value, json};

use crate::js;
use crate::regex_js;
use crate::titles::{self, clean, clean_v, ou};

/// `KALSHI_TITLE_FORMAT_VERSION`.
pub const VERSAO_TITULO: i64 = 4;

fn campo<'a>(v: Option<&'a Value>, k: &str) -> Option<&'a Value> {
    v.and_then(|v| v.get(k))
}

fn texto_truthy(v: Option<&Value>) -> String {
    if js::truthy(v) { js::string(v) } else { String::new() }
}

regex_js!(NT_REG_TIME, r"(?i)^Reg Time:\s*");
regex_js!(NT_REGULATION, r"(?i)^Regulation Time:\s*");
regex_js!(NT_SIM_NAO, r"(?i)^(Yes|No)\s*[:\-]?\s*");
regex_js!(NT_VENCE, r"(?i)\s+(wins?|to win|is the winner)\.?$");
regex_js!(CONFRONTO_TEXTO, r"(?i)(.+?)\s+(?:vs\.?|versus|@)\s+(.+?)(?:\s*[:\-]|$)");

/// `nomeTimeKalshi()`.
pub fn nome_time(v: &str) -> String {
    let s = clean(v);
    let s = NT_REG_TIME.replace(&s, "");
    let s = NT_REGULATION.replace(&s, "");
    let s = NT_SIM_NAO.replace(&s, "");
    let s = NT_VENCE.replace(&s, "");
    clean(&s)
}

/// `extrairConfrontoDeTexto()`: primeiro "A vs B" encontrado nos textos.
pub fn extrair_confronto(valores: &[Option<&Value>]) -> String {
    for bruto in valores {
        let s = clean_v(*bruto);
        if s.is_empty() {
            continue;
        }
        if let Some(c) = CONFRONTO_TEXTO.captures(&s) {
            let (a, b) = (nome_time(&c[1]), nome_time(&c[2]));
            if !a.is_empty() && !b.is_empty() && a.to_lowercase() != b.to_lowercase() {
                return format!("{a} vs {b}");
            }
        }
    }
    String::new()
}

fn data_ou_zero(v: Option<&Value>) -> f64 {
    js::data_ms(&texto_truthy(v)).unwrap_or(0.0)
}

fn contem_texto(lista: Option<&Value>, alvo: &str) -> bool {
    lista.and_then(Value::as_array).is_some_and(|a| a.iter().any(|v| v.as_str() == Some(alvo)))
}

/// `escolherMilestone()`: milestone primário (ou relacionado) do evento exato,
/// o mais recentemente atualizado.
pub fn escolher_milestone(milestones: Option<&Value>, ticker_evento: &str) -> Value {
    let lista: Vec<&Value> =
        milestones.and_then(Value::as_array).map(|a| a.iter().filter(|m| js::truthy(Some(m))).collect()).unwrap_or_default();
    let primarios: Vec<&Value> = lista.iter().copied().filter(|m| contem_texto(m.get("primary_event_tickers"), ticker_evento)).collect();
    let relacionados: Vec<&Value> = lista.iter().copied().filter(|m| contem_texto(m.get("related_event_tickers"), ticker_evento)).collect();
    let mut pool = if !primarios.is_empty() { primarios } else { relacionados };
    pool.sort_by(|a, b| {
        let (au, bu) = (data_ou_zero(a.get("last_updated_ts")), data_ou_zero(b.get("last_updated_ts")));
        if bu != au {
            return bu.total_cmp(&au);
        }
        data_ou_zero(b.get("start_date")).total_cmp(&data_ou_zero(a.get("start_date")))
    });
    pool.first().map_or(Value::Null, |m| (*m).clone())
}

regex_js!(TIPO_JOGO, r"(?i)_(football|basketball|soccer|hockey|baseball|tennis)_?game");

/// `isSportsEvent()`.
pub fn e_evento_esportivo(evento: Option<&Value>) -> bool {
    texto_truthy(campo(evento, "category")).to_lowercase() == "sports"
        || TIPO_JOGO.is_match(&texto_truthy(campo(campo(evento, "_milestone"), "type")))
}

regex_js!(ES_AMERICANO, r"american_?football|\bnfl\b|\bncaaf\b");

/// `esporteKalshiEvento()`: modalidade pelo tipo oficial do milestone.
pub fn esporte_evento(evento: Option<&Value>) -> Value {
    let mut tipos: Vec<String> = Vec::new();
    let principal = campo(campo(evento, "_milestone"), "type");
    if js::truthy(principal) {
        tipos.push(js::string(principal));
    }
    if let Some(lista) = campo(evento, "_milestones").and_then(Value::as_array) {
        for m in lista {
            let t = m.get("type");
            if js::truthy(t) {
                tipos.push(js::string(t));
            }
        }
    }
    let t = tipos.join(" ").to_lowercase();
    let nome = if t.contains("soccer") {
        "Soccer"
    } else if ES_AMERICANO.is_match(&t) {
        "American Football"
    } else if t.contains("basketball") {
        "Basketball"
    } else if t.contains("baseball") {
        "Baseball"
    } else if t.contains("hockey") {
        "Hockey"
    } else if t.contains("tennis") {
        "Tennis"
    } else if t.contains("golf") {
        "Golf"
    } else if t.contains("cricket") {
        "Cricket"
    } else if t.contains("rugby") {
        "Rugby"
    } else {
        return campo(evento, "category").filter(|v| js::truthy(Some(v))).cloned().unwrap_or(Value::Null);
    };
    Value::String(nome.into())
}

/// `metadadoEsporteKalshi()`: `[sport, _sportMetaResolved, _sportMetaChecked]`.
pub fn metadado_esporte(evento: Option<&Value>, mercado: Option<&Value>) -> [(&'static str, Value); 3] {
    let mut esporte = esporte_evento(evento);
    if !js::truthy(Some(&esporte)) || js::string(Some(&esporte)).to_lowercase() == "sports" {
        if let Some(regras) = titles::sport_from_kalshi_rules(mercado) {
            esporte = Value::String(regras.into());
        }
    }
    let tem_evento = evento.is_some_and(|e| js::truthy(Some(e)));
    let tem_esporte = js::truthy(Some(&esporte));
    if !tem_evento && !tem_esporte {
        return [("sport", Value::Null), ("_sportMetaResolved", json!(false)), ("_sportMetaChecked", json!(false))];
    }
    let resolvido = tem_esporte && js::string(Some(&esporte)).to_lowercase() != "sports";
    [("sport", esporte), ("_sportMetaResolved", json!(resolvido)), ("_sportMetaChecked", json!(tem_evento || tem_esporte))]
}

/// `eventoKalshiTemMilestoneEsportivo()`.
pub fn tem_milestone_esportivo(evento: Option<&Value>) -> bool {
    texto_truthy(campo(evento, "category")).to_lowercase() != "sports"
        || js::truthy(campo(campo(evento, "_milestone"), "type"))
        || campo(evento, "_milestones").and_then(Value::as_array).is_some_and(|a| a.iter().any(|m| js::truthy(m.get("type"))))
}

/// `Number(x.toFixed(4))` com o arredondamento do JS (empate para cima).
fn to_fixed4(n: f64) -> f64 {
    let a = n.abs();
    let exato = format!("{a:.30}");
    let resto = exato.split_once('.').map_or("", |(_, f)| f).get(4..).unwrap_or("");
    let empate = resto.starts_with('5') && resto[1..].bytes().all(|b| b == b'0');
    let base = if empate { a + 1e-9 } else { a };
    let r: f64 = format!("{base:.4}").parse().unwrap_or(f64::NAN);
    if n < 0.0 { -r } else { r }
}

/// `numeroStrike()`.
fn numero_strike(v: Option<&Value>) -> Option<String> {
    let n = js::number(v);
    if !n.is_finite() {
        return None;
    }
    let valor = if n.fract() == 0.0 { n } else { to_fixed4(n) };
    js::numero_texto(valor)
}

fn textos_mercado(m: Option<&Value>) -> Vec<String> {
    ["title", "subtitle", "yes_sub_title", "no_sub_title"].iter().map(|k| clean_v(campo(m, k))).filter(|s| !s.is_empty()).collect()
}

regex_js!(IT_OVER_UNDER, r"(?i)\b(?:over|under|above|below|at\s+least|at\s+most)\s+\d+(?:[.,]\d+)?\b");
regex_js!(IT_OU, r"(?i)\bo\s*/\s*u\s*\d+(?:[.,]\d+)?\b");
regex_js!(IT_TOTAL, r"(?i)\btotal\s+(?:goals?|runs?|points?)\b");
regex_js!(IT_LADO, r"(?i)^(?:over|under)$");

/// `mercadoKalshiIndicaTotal()`.
pub fn indica_total(m: Option<&Value>, texto_lado: &str) -> bool {
    let texto = textos_mercado(m).join(" ");
    IT_OVER_UNDER.is_match(&texto) || IT_OU.is_match(&texto) || IT_TOTAL.is_match(&texto) || IT_LADO.is_match(texto_lado)
}

regex_js!(PB_INTERROGACOES, r"\?+$");
regex_js!(PB_SIM_NAO, r"(?i)^(?:yes|no)$");

/// `perguntaBinariaKalshi()`: a pergunta, quando o mercado é Yes/No genérico.
pub fn pergunta_binaria(m: Option<&Value>) -> String {
    let texto_mercado = clean_v(ou(campo(m, "title"), campo(m, "subtitle")));
    let sim = clean_v(campo(m, "yes_sub_title"));
    let nao = clean_v(campo(m, "no_sub_title"));
    let bruta = match texto_mercado.rfind(':') {
        Some(i) => &texto_mercado[i + 1..],
        None => texto_mercado.as_str(),
    };
    let pergunta = PB_INTERROGACOES.replace(bruta, "").trim().to_string();
    let lados_genericos = PB_SIM_NAO.is_match(&sim) && PB_SIM_NAO.is_match(&nao);
    if lados_genericos && !pergunta.is_empty() && !indica_total(m, "") && !PB_SIM_NAO.is_match(&pergunta) {
        pergunta
    } else {
        String::new()
    }
}

regex_js!(PI_PERGUNTA, r"(?i)^(?:first|1st)\s+inning\s+runs?$");

/// `totalPrimeiroInningKalshi()`.
fn total_primeiro_inning(m: Option<&Value>, lado: Option<&str>) -> String {
    let pergunta = pergunta_binaria(m);
    let piso = js::number(campo(m, "floor_strike"));
    let tipo = texto_truthy(campo(m, "strike_type")).to_lowercase();
    if !PI_PERGUNTA.is_match(&pergunta) || piso != 1.0 || tipo != "greater_or_equal" {
        return String::new();
    }
    let ou = match lado {
        Some("yes") => "Over",
        Some("no") => "Under",
        _ => "Over/Under",
    };
    format!("{ou} 0.5 runs in the 1st inning")
}

regex_js!(DM_SIM_NAO, r"(?i)^(yes|no)$");
regex_js!(DM_TOTAL_UNIDADE, r"(?i)^(?:will\s+)?(over|under)\s+(\d+(?:[.,]\d+)?)\s+(goals?|runs?|points?)(?:\s+be\s+scored)?\?*$");
regex_js!(DM_GOL, r"(?i)^goal");
regex_js!(DM_RUN, r"(?i)^run");
regex_js!(DM_OVER_UNDER, r"(?i)^(over|under)$");
regex_js!(DM_MAIOR, r"(?i)greater|above|over");
regex_js!(DM_MENOR, r"(?i)less|below|under");

/// Escopo do evento: `competition_scope || title || ''`.
fn escopo_evento(evento: Option<&Value>) -> String {
    let escopo = ou(campo(campo(evento, "product_metadata"), "competition_scope"), campo(evento, "title"));
    texto_truthy(escopo)
}

/// `detalheMercadoKalshi()`.
pub fn detalhe_mercado(m: Option<&Value>, lado: Option<&str>, evento: Option<&Value>) -> String {
    let estruturado = m.map_or(String::new(), |m| titles::kalshi_detail(m, lado, &escopo_evento(evento)));
    if !estruturado.is_empty() {
        return estruturado;
    }
    let texto_lado = clean_v(campo(m, if lado == Some("no") { "no_sub_title" } else { "yes_sub_title" }));
    let lado_limpo = if !texto_lado.is_empty() && !DM_SIM_NAO.is_match(&texto_lado) { texto_lado.clone() } else { String::new() };
    let piso = numero_strike(campo(m, "floor_strike"));
    let teto = numero_strike(campo(m, "cap_strike"));
    let tipo = texto_truthy(campo(m, "strike_type")).to_lowercase();
    let strike = match (&piso, &teto) {
        (Some(p), Some(t)) if p != t => format!("{p}–{t}"),
        (Some(p), _) => p.clone(),
        (None, Some(t)) => t.clone(),
        (None, None) => String::new(),
    };
    // Alguns mercados de gols só dizem "over/under X goals" no título.
    let texto_mercado = clean_v(ou(campo(m, "title"), campo(m, "subtitle")));
    let total = indica_total(m, &texto_lado);
    if let Some(c) = DM_TOTAL_UNIDADE.captures(&texto_mercado) {
        let unidade = if DM_GOL.is_match(&c[3]) {
            "gols"
        } else if DM_RUN.is_match(&c[3]) {
            "runs"
        } else {
            "pontos"
        };
        return format!("{} {} {unidade}", titles::capitalizar(&c[1]), c[2].replacen(',', ".", 1));
    }
    // floor_strike num mercado binário não é necessariamente um total.
    let binaria = pergunta_binaria(m);
    if !binaria.is_empty() {
        let primeiro = total_primeiro_inning(m, lado);
        if !primeiro.is_empty() {
            return primeiro;
        }
        return match lado {
            Some("yes") => format!("{binaria}: Yes"),
            Some("no") => format!("{binaria}: No"),
            _ => binaria,
        };
    }
    if !lado_limpo.is_empty() {
        if DM_OVER_UNDER.is_match(&lado_limpo) && !strike.is_empty() {
            return format!("{} {strike}", titles::capitalizar(&lado_limpo));
        }
        return lado_limpo;
    }
    if !strike.is_empty() && total {
        if DM_MAIOR.is_match(&tipo) {
            return format!("Over {strike}");
        }
        if DM_MENOR.is_match(&tipo) {
            return format!("Under {strike}");
        }
        return strike;
    }
    String::new()
}

regex_js!(KT_VS, r"(?i)\b(?:vs\.?|versus|@)\b");

/// `kalshiTitle()`: "confronto - aposta" para um mercado Kalshi.
pub fn kalshi_title(m: Option<&Value>, evento: Option<&Value>, ticker: &str, lado: Option<&str>) -> String {
    let m = m.filter(|m| js::truthy(Some(m)));
    let evento = evento.filter(|e| js::truthy(Some(e)));
    if m.is_none() && evento.is_none() {
        return ticker.to_string();
    }
    let times = campo(evento, "_teamsTitle");
    let confronto = titles::clean_kalshi_matchup(&clean_v(times));
    let titulo_evento = clean_v(ou(ou(campo(evento, "title"), campo(evento, "sub_title")), campo(evento, "subtitle")));
    let times_texto = clean_v(times);
    let qualificar = |aposta: &str| titles::kalshi_bet_qualifier(aposta, &[&titulo_evento, &times_texto]);
    let titulo_mercado = clean_v(ou(ou(ou(campo(m, "title"), campo(m, "subtitle")), campo(m, "yes_sub_title")), campo(m, "no_sub_title")));
    let base = if confronto.is_empty() {
        titles::clean_kalshi_matchup(titulo_evento.split(':').next().unwrap_or(""))
    } else {
        confronto.clone()
    };
    let titulo_sintetico = format!("{base} - {titulo_mercado}");
    let escopo = {
        let competicao = campo(campo(evento, "product_metadata"), "competition_scope");
        if js::truthy(competicao) { js::string(competicao) } else { titulo_evento.clone() }
    };
    let vazio = json!({});
    let especial = titles::kalshi_special_title(&titles::TituloEspecial {
        ticker,
        lado,
        titulo: &titulo_sintetico,
        mercado: m.unwrap_or(&Value::Null),
        escopo: &escopo,
    });
    if !especial.is_empty() {
        return qualificar(&especial);
    }
    let entrada = titles::inning_winner_title(ticker, lado.unwrap_or(""), &titulo_sintetico, Some(m.unwrap_or(&vazio)));
    if !entrada.is_empty() {
        return qualificar(&entrada);
    }
    let detalhe = clean(&detalhe_mercado(m, lado, evento));
    let binaria = pergunta_binaria(m);
    // Numa pergunta binária, o título oficial do evento vale mais que a ordem home/away.
    let oficial = titles::clean_kalshi_matchup(titulo_evento.split(':').next().unwrap_or(""));
    let confronto_binario = if KT_VS.is_match(&oficial) { oficial.clone() } else { confronto.clone() };
    if !binaria.is_empty() && !confronto_binario.is_empty() {
        return format!("{confronto_binario} - {}", qualificar(&detalhe));
    }
    if !confronto.is_empty() && !detalhe.is_empty() {
        return format!("{confronto} - {}", qualificar(&detalhe));
    }
    // Sem estruturar o mercado, preserva o texto oficial da Kalshi.
    if !confronto.is_empty() && !titulo_mercado.is_empty() && titulo_mercado.to_lowercase() != confronto.to_lowercase() {
        let padrao = format!(r"(?i)^{}\s*(?:[-–—:]\s*)?", regex::escape(&confronto));
        let complemento = regex::Regex::new(&padrao)
            .map(|r| r.replace(&titulo_mercado, "").trim().to_string())
            .unwrap_or_else(|_| titulo_mercado.clone());
        if !complemento.is_empty() {
            return format!("{confronto} - {}", qualificar(&complemento));
        }
    }
    [titulo_evento, titulo_mercado, confronto].into_iter().find(|s| !s.is_empty()).unwrap_or_else(|| ticker.to_string())
}

regex_js!(ISO_NUMERICO, r"^\d+(?:\.\d+)?$");

/// `toIsoKalshiDate()`.
pub fn para_iso(v: Option<&Value>) -> Option<String> {
    let v = v?;
    match v {
        Value::Null => None,
        Value::String(s) if s.is_empty() => None,
        Value::Number(n) => {
            let n = n.as_f64()?;
            js::iso(if n < 1e12 { n * 1000.0 } else { n })
        }
        _ => {
            let texto = js::string(Some(v));
            if ISO_NUMERICO.is_match(texto.trim()) {
                let n = js::number(Some(v));
                js::iso(if n < 1e12 { n * 1000.0 } else { n })
            } else {
                js::data_ms(&texto).and_then(js::iso)
            }
        }
    }
}

/// `dataKalshiCompativelComLiquidacao()`: uma aposta não começa depois de liquidada.
pub fn compativel_com_liquidacao(data: Option<&Value>, liquidacao: Option<&Value>) -> bool {
    match (js::data_ms(&texto_truthy(data)), js::data_ms(&texto_truthy(liquidacao))) {
        (Some(inicio), Some(fim)) => inicio <= fim,
        _ => true,
    }
}

/// `kalshiStartDate()`: primeira data oficial compatível com a liquidação.
pub fn data_inicio(m: Option<&Value>, evento: Option<&Value>, liquidacao: Option<&Value>) -> Option<String> {
    let milestone = campo(campo(evento, "_milestone"), "start_date");
    let fontes: Vec<Option<&Value>> = if e_evento_esportivo(evento) {
        vec![milestone, campo(m, "occurrence_datetime"), campo(evento, "_occurrenceDate"), campo(evento, "_startDate"), campo(evento, "strike_date")]
    } else {
        vec![
            campo(evento, "_startDate"),
            milestone,
            campo(m, "occurrence_datetime"),
            campo(evento, "_occurrenceDate"),
            campo(evento, "strike_date"),
            campo(m, "target_datetime"),
            campo(m, "start_date"),
        ]
    };
    fontes.into_iter().find_map(|f| {
        let data = para_iso(f)?;
        compativel_com_liquidacao(Some(&Value::String(data.clone())), liquidacao).then_some(data)
    })
}

/// `chaveLiquidacaoKalshi()`.
pub fn chave_liquidacao(s: &Value) -> String {
    let id = ou(ou(s.get("settlement_id"), s.get("settlementId")), s.get("id"));
    if js::truthy(id) {
        return format!("id:{}", js::string(id));
    }
    let ticker = texto_truthy(ou(s.get("ticker"), s.get("market_ticker")));
    let quando = texto_truthy(ou(ou(s.get("settled_time"), s.get("settlement_time")), s.get("updated_time")));
    format!("{ticker}|{quando}")
}

/// `assinaturaLiquidacaoKalshi()`.
pub fn assinatura_liquidacao(s: &Value) -> String {
    ["market_result", "revenue", "yes_total_cost_dollars", "no_total_cost_dollars", "fee_cost", "settled_time", "updated_time"]
        .iter()
        .map(|k| texto_truthy(s.get(*k)))
        .collect::<Vec<_>>()
        .join("|")
}

/// `liquidacaoKalshiIncompleta()`.
pub fn liquidacao_incompleta(linha: Option<&Value>) -> bool {
    let Some(l) = linha.filter(|l| js::truthy(Some(l))) else { return true };
    l.get("_feeCost").is_none_or(Value::is_null)
        || ["title", "slug", "outcome", "_startDate"].iter().any(|k| !js::truthy(l.get(*k)))
}

/// `dataInicioKalshiInvalida()`.
pub fn data_inicio_invalida(linha: Option<&Value>) -> bool {
    match linha.filter(|l| js::truthy(Some(l))) {
        None => true,
        Some(l) => !compativel_com_liquidacao(l.get("_startDate"), l.get("_settledTime")),
    }
}

regex_js!(TR_SIM_NAO_FINAL, r"(?i):\s*(Yes|No)$");
regex_js!(TR_OVER_UNDER, r"(?i)\s-\s(?:over|under)\s+\d+(?:[.,]\d+)?\s*$");
regex_js!(TR_PRIMEIRO_INNING, r"(?i)\s-\s(?:first|1st)\s+inning\s+runs?(?:\s*:\s*(?:yes|no))?\s*$");

/// `tituloKalshiPrecisaRevisar()`: títulos de versões antigas que precisam ser refeitos.
pub fn titulo_precisa_revisar(linha: &Value) -> bool {
    if js::number(linha.get("_kalshiTitleFormatVersion")) >= VERSAO_TITULO as f64 {
        return false;
    }
    let titulo = clean_v(linha.get("title"));
    let detalhe = titulo.split(" - ").skip(1).collect::<Vec<_>>().join(" - ");
    let detalhe = TR_SIM_NAO_FINAL.replace(&detalhe, "").trim().to_string();
    if !titles::compact_kalshi_total(&detalhe, &clean_v(linha.get("outcome")), "").is_empty() {
        return true;
    }
    TR_OVER_UNDER.is_match(&titulo) || TR_PRIMEIRO_INNING.is_match(&titulo)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Compara com o JS original. `BETTY_ORACULO_TITULOS=<jsonl>` e
    /// `BETTY_ORACULO_DB=<banco>` (de onde vêm mercados e eventos).
    #[test]
    #[ignore]
    fn oraculo_titulos() {
        let var = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("defina {k}"));
        let c = rusqlite::Connection::open_with_flags(var("BETTY_ORACULO_DB"), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let meta = |kind: &str| -> HashMap<String, Value> {
            let mut s = c.prepare("SELECT cache_key, data_json FROM kalshi_metadata_cache WHERE kind=?").unwrap();
            s.query_map([kind], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .unwrap()
                .map(|r| r.unwrap())
                .map(|(k, d)| (k, serde_json::from_str(&d).unwrap()))
                .collect()
        };
        let (mercados, eventos) = (meta("market"), meta("event"));
        let texto = std::fs::read_to_string(var("BETTY_ORACULO_TITULOS")).unwrap();
        let s = |v: &Value| v.as_str().unwrap_or("").to_string();
        let (mut total, mut erros, mut por_fn) = (0usize, Vec::new(), HashMap::<String, usize>::new());
        for linha in texto.lines() {
            let caso: Value = serde_json::from_str(linha).unwrap();
            let f = caso["fn"].as_str().unwrap();
            if caso.get("erro").is_some() {
                continue; // o JS lançou exceção: fora do contrato
            }
            let a = &caso["args"];
            let lado = caso.get("lado").and_then(Value::as_str);
            let m = caso.get("ticker").and_then(Value::as_str).and_then(|t| mercados.get(t));
            let ev = m.and_then(|m| m.get("event_ticker")).and_then(Value::as_str).and_then(|t| eventos.get(t));
            let ev_proprio = caso.get("evento").and_then(Value::as_str).and_then(|t| eventos.get(t));
            let ticker = caso.get("ticker").and_then(Value::as_str).unwrap_or("");
            let rust: Value = match f {
                "kalshiTitle" => json!(kalshi_title(m, ev, ticker, lado)),
                "kalshiTitleSemEvento" => json!(kalshi_title(m, None, ticker, lado)),
                "detalheMercadoKalshi" => json!(detalhe_mercado(m, lado, ev)),
                "kalshiDetail" => json!(titles::kalshi_detail(m.unwrap(), lado, &escopo_evento(ev))),
                "perguntaBinariaKalshi" => json!(pergunta_binaria(m)),
                "kalshiStartDate" => json!(data_inicio(m, ev, None)),
                "kalshiStartDateLiq" => json!(data_inicio(m, ev, m.and_then(|m| m.get("close_time")).filter(|v| js::truthy(Some(v))))),
                "metadadoEsporteKalshi" => Value::Object(metadado_esporte(ev, m).into_iter().map(|(k, v)| (k.to_string(), v)).collect()),
                "sportFromKalshiRules" => json!(titles::sport_from_kalshi_rules(m)),
                "isSportsEvent" => json!(e_evento_esportivo(ev_proprio)),
                "eventoKalshiTemMilestoneEsportivo" => json!(tem_milestone_esportivo(ev_proprio)),
                "escolherMilestone" => escolher_milestone(ev_proprio.and_then(|e| e.get("_milestones")), caso["evento"].as_str().unwrap()),
                "extrairConfrontoDeTexto" => match ev_proprio {
                    Some(e) => {
                        let mut v: Vec<Option<&Value>> = vec![e.get("title"), e.get("sub_title"), e.get("subtitle"), e.get("_milestone").and_then(|m| m.get("title"))];
                        if let Some(ms) = e.get("_markets").and_then(Value::as_array) {
                            for mm in ms {
                                v.extend([mm.get("title"), mm.get("subtitle"), mm.get("yes_sub_title"), mm.get("no_sub_title")]);
                            }
                        }
                        json!(extrair_confronto(&v))
                    }
                    None => json!(extrair_confronto(&a[0].as_array().unwrap().iter().map(Some).collect::<Vec<_>>())),
                },
                "tituloKalshiPrecisaRevisar" => json!(titulo_precisa_revisar(&a[0])),
                "kalshiTitleMissingMatchup" => json!(titles::kalshi_title_missing_matchup(&a[0], None)),
                "liquidacaoKalshiIncompleta" => json!(liquidacao_incompleta(Some(&a[0]))),
                "dataInicioKalshiInvalida" => json!(data_inicio_invalida(Some(&a[0]))),
                "period" => json!(titles::period(&s(&a[0]))),
                "unit" => json!(titles::unit(&s(&a[0]), "")),
                "cleanKalshiMatchup" => json!(titles::clean_kalshi_matchup(&s(&a[0]))),
                "selectionDetail" => json!(titles::selection_detail(&s(&a[0]), &s(&a[1]), &s(&a[2]))),
                "compactKalshiTotal" => json!(titles::compact_kalshi_total(&s(&a[0]), &s(&a[1]), &s(&a[2]))),
                "drawScope" => json!(titles::draw_scope(&s(&a[0]))),
                "drawDetail" => json!(titles::draw_detail(&s(&a[0]), &s(&a[1]), &s(&a[2]))),
                "kalshiBetQualifier" => json!(titles::kalshi_bet_qualifier(&s(&a[0]), &[&s(&a[1]), &s(&a[2])])),
                "toIsoKalshiDate" => json!(para_iso(Some(&a[0]))),
                outro => panic!("função desconhecida {outro}"),
            };
            total += 1;
            let esperado = &caso["out"];
            if &rust != esperado {
                *por_fn.entry(f.to_string()).or_default() += 1;
                if erros.len() < 40 {
                    erros.push(format!("{f} {}\n   rust {rust}\n   js   {esperado}", if a.is_null() { format!("{ticker} {lado:?}") } else { a.to_string().chars().take(160).collect() }));
                }
            }
        }
        for e in &erros {
            eprintln!("{e}");
        }
        eprintln!("divergências por função: {por_fn:?}");
        assert!(por_fn.is_empty(), "{} de {total} divergem", por_fn.values().sum::<usize>());
        eprintln!("{total} casos idênticos ao JS");
    }

    #[test]
    fn strike_e_datas() {
        assert_eq!(numero_strike(Some(&json!(2.5))).as_deref(), Some("2.5"));
        assert_eq!(numero_strike(Some(&json!(3))).as_deref(), Some("3"));
        assert_eq!(numero_strike(Some(&Value::Null)).as_deref(), Some("0"), "Number(null) = 0 no JS");
        assert_eq!(numero_strike(None), None);
        assert_eq!(to_fixed4(1.03125), 1.0313, "empate arredonda para cima como o toFixed");
        assert_eq!(para_iso(Some(&json!(1795215600))).as_deref(), Some("2026-11-20T23:00:00.000Z"));
        assert_eq!(para_iso(Some(&json!("2026-11-20T23:00:00Z"))).as_deref(), Some("2026-11-20T23:00:00.000Z"));
        assert!(!compativel_com_liquidacao(Some(&json!("2026-11-21T00:00:00Z")), Some(&json!("2026-11-20T00:00:00Z"))));
        assert_eq!(chave_liquidacao(&json!({"ticker": "KX-1", "settled_time": "t"})), "KX-1|t");
        assert_eq!(assinatura_liquidacao(&json!({"revenue": 0, "market_result": "yes"})), "yes||||||");
    }
}
