//! Títulos de mercado, igual a `public/market-title-utils.js`: período,
//! unidade, seleções especiais (Race, First TD, empate), totais compactos e o
//! detalhe estruturado dos mercados Kalshi. Só apresentação: nunca altera
//! valores financeiros.

use serde_json::Value;
use unicode_normalization::UnicodeNormalization;

use crate::js;
use crate::regex_exata;

/// `clean()`: `String(v || '')` com espaços normalizados.
pub fn clean(s: &str) -> String {
    js::limpar(s)
}

pub fn clean_v(v: Option<&Value>) -> String {
    if js::truthy(v) { clean(&js::texto(v)) } else { String::new() }
}

/// `a || b` sobre campos de um objeto.
pub fn ou<'a>(a: Option<&'a Value>, b: Option<&'a Value>) -> Option<&'a Value> {
    if js::truthy(a) { a } else { b }
}

pub fn ordinal(n: i64) -> String {
    let ultimo = n % 100;
    let sufixo = if (11..=13).contains(&ultimo) {
        "th"
    } else {
        match n % 10 {
            1 => "st",
            2 => "nd",
            3 => "rd",
            _ => "th",
        }
    };
    format!("{n}{sufixo}")
}

fn ordinal_texto(s: &str) -> String {
    ordinal(s.parse::<i64>().unwrap_or(0))
}

/// Primeira letra maiúscula e o resto minúsculo.
pub fn capitalizar(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(p) => p.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(),
        None => String::new(),
    }
}

/// Texto de um grupo de captura, ou vazio quando o grupo não participou.
fn grupo<'a>(c: &regex::Captures<'a>, i: usize) -> &'a str {
    c.get(i).map_or("", |m| m.as_str())
}

fn junta_nao_vazios(partes: &[&str]) -> String {
    partes.iter().filter(|p| !p.is_empty()).copied().collect::<Vec<_>>().join(" ")
}

const ORDINAIS: [&str; 9] = ["first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth", "ninth"];

regex_exata!(P_CINCO_INNINGS, r"(?i)\b(?:first|1st)\s*(?:5|five)\s+innings?\b");
regex_exata!(P_FAIXA_INNINGS, r"(?i)\b(?:first|1st)\s+(\d+)\s+innings\b");
regex_exata!(P_INNING, r"(?i)\b(\d+)(?:st|nd|rd|th|º|°)\s+inning\b");
regex_exata!(P_INNING_NOME, r"(?i)\b(first|second|third|fourth|fifth|sixth|seventh|eighth|ninth)\s+inning\b");
regex_exata!(P_QUARTO, r"(?i)\b([1-4])Q\b|\b([1-4])(?:st|nd|rd|th)\s+quarter\b|(?:^|_)q([1-4])(?:_|$)|\b([1-4])[º°]\s+quarto\b");
regex_exata!(P_TEMPO1, r"(?i)\b(?:first|1st)\s+half\b|\b1H\b|(?:first|1st)_half");
regex_exata!(P_TEMPO2, r"(?i)\b(?:second|2nd)\s+half\b|\b2H\b|(?:second|2nd)_half");

/// `period()`: período do jogo citado no texto.
pub fn period(texto: &str) -> String {
    let s = clean(texto);
    if P_CINCO_INNINGS.is_match(&s) {
        return "1st 5 Innings".into();
    }
    if let Some(c) = P_FAIXA_INNINGS.captures(&s) {
        return format!("1st {} Innings", c[1].parse::<u64>().unwrap_or(0));
    }
    if let Some(c) = P_INNING.captures(&s) {
        return format!("{} Inning", ordinal_texto(&c[1]));
    }
    if let Some(c) = P_INNING_NOME.captures(&s) {
        let i = ORDINAIS.iter().position(|o| *o == c[1].to_lowercase()).map_or(0, |i| i + 1);
        return format!("{} Inning", ordinal(i as i64));
    }
    if let Some(c) = P_QUARTO.captures(&s) {
        let numero = [1, 2, 3, 4].iter().find_map(|&i| c.get(i)).map_or("", |m| m.as_str());
        return if numero == "1" { "1st Quarter".into() } else { format!("{numero}º quarto") };
    }
    if P_TEMPO1.is_match(&s) {
        return "1st Half".into();
    }
    if P_TEMPO2.is_match(&s) {
        return "2nd Half".into();
    }
    String::new()
}

regex_exata!(QUARTO_PT, r"^([1-4])º quarto$");

/// `period(x).replace(/^([1-4])º quarto$/, ordinal Quarter)`.
fn period_quarter(texto: &str) -> String {
    let p = period(texto);
    match QUARTO_PT.captures(&p) {
        Some(c) => format!("{} Quarter", ordinal_texto(&c[1])),
        None => p,
    }
}

regex_exata!(U_ESCANTEIOS, r"(?i)\b(?:corners?|corner kicks?|escanteios)\b");
regex_exata!(U_REBOTES, r"(?i)\b(?:rebounds?|rebotes)\b");
regex_exata!(U_ROUNDS, r"(?i)\brounds?\b");
regex_exata!(U_GOLS, r"(?i)\b(?:goals?|gols)\b");
regex_exata!(U_RUNS, r"(?i)\b(?:runs?|baseball|mlb)\b");
regex_exata!(U_PONTOS, r"(?i)\b(?:points?|pontos|basketball|basquete|american football|nfl)\b");
regex_exata!(U_SLUG_PONTOS, r"(?i)^(?:nba|wnba|ncaab|cbb|bkbsl|fiba|euroleague|cfb|ncaaf|nfl|cfl|ufl|xfl)-");
regex_exata!(U_SLUG_RUNS, r"(?i)^(?:mlb|npb|kbo|cpbl|milb)-");

/// `unit()`: unidade de contagem citada no texto (ou deduzida do slug).
pub fn unit(texto: &str, slug: &str) -> &'static str {
    let s = clean(texto);
    if U_ESCANTEIOS.is_match(&s) {
        "escanteios"
    } else if U_REBOTES.is_match(&s) {
        "rebotes"
    } else if U_ROUNDS.is_match(&s) {
        "rounds"
    } else if U_GOLS.is_match(&s) {
        "gols"
    } else if U_RUNS.is_match(&s) {
        "runs"
    } else if U_PONTOS.is_match(&s) || U_SLUG_PONTOS.is_match(slug) {
        "pontos"
    } else if U_SLUG_RUNS.is_match(slug) {
        "runs"
    } else {
        ""
    }
}

regex_exata!(R_AMERICANO, r"(?i)\b(?:american|college|pro|professional)\s+football\s+game\b");
regex_exata!(R_BASQUETE, r"(?i)\bbasketball\s+(?:game|match)\b");
regex_exata!(R_BEISEBOL, r"(?i)\bbaseball\s+(?:game|match)\b");
regex_exata!(R_HOQUEI, r"(?i)\b(?:ice\s+)?hockey\s+(?:game|match)\b");
regex_exata!(R_TENIS, r"(?i)\btennis\s+(?:game|match)\b");
regex_exata!(R_FUTEBOL, r"(?i)\bsoccer\s+(?:game|match)\b");
regex_exata!(R_RUGBY, r"(?i)\brugby\s+(?:game|match)\b");
regex_exata!(R_CRIQUETE, r"(?i)\bcricket\s+(?:game|match)\b");
regex_exata!(R_VOLEI, r"(?i)\bvolleyball\s+(?:game|match)\b");

/// `sportFromKalshiRules()`: só uma modalidade explícita de game/match conta.
pub fn sport_from_kalshi_rules(mercado: Option<&Value>) -> Option<&'static str> {
    let regras = clean_v(mercado.and_then(|m| m.get("rules_primary")));
    if regras.is_empty() {
        return None;
    }
    let padroes: [(&str, &regex::Regex); 9] = [
        ("American Football", &R_AMERICANO),
        ("Basketball", &R_BASQUETE),
        ("Baseball", &R_BEISEBOL),
        ("Hockey", &R_HOQUEI),
        ("Tennis", &R_TENIS),
        ("Soccer", &R_FUTEBOL),
        ("Rugby", &R_RUGBY),
        ("Cricket", &R_CRIQUETE),
        ("Volleyball", &R_VOLEI),
    ];
    let achados: Vec<&str> = padroes.iter().filter(|(_, p)| p.is_match(&regras)).map(|(n, _)| *n).collect();
    (achados.len() == 1).then(|| achados[0])
}

/// `footballWinner()`.
pub fn football_winner(time: &str, escopo: &str, lado: Option<&str>) -> String {
    let p = period(escopo);
    let p = match QUARTO_PT.captures(&p) {
        Some(c) => {
            let n: usize = c[1].parse().unwrap_or(0);
            format!("{n}{} Quarter", ["", "st", "nd", "rd", "th"][n])
        }
        None => p,
    };
    let time = clean(time);
    junta_nao_vazios(&[&p, &time, "ML", if lado == Some("no") { "No" } else { "" }])
}

regex_exata!(INNING_WIN_TICKER, r"^KX(?:MLB|NPB|KBO|CPBL|MILB|WBC)INNINGWIN-[^-]+-([1-9]\d*)-([A-Z0-9]+)$");
regex_exata!(VS_SPLIT, r"(?i)\s+vs\.?\s+");
regex_exata!(WINS_FINAL, r"(?i)\s+wins?\??$");
regex_exata!(INNING_PREFIXO, r"(?i)^\d+(?:st|nd|rd|th) Inning\s+");
regex_exata!(ML_FINAL, r"(?i)\s+ML(?:\s+No)?$");
regex_exata!(NO_FINAL, r"(?i)\s+No$");

fn so_alfanumerico_minusculo(s: &str) -> String {
    clean(s).to_lowercase().chars().filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit()).collect()
}

/// `inningWinnerTitle()`: vencedor de uma entrada específica (beisebol Kalshi).
pub fn inning_winner_title(ticker: &str, lado: &str, titulo: &str, mercado: Option<&Value>) -> String {
    let ticker = ticker.to_uppercase();
    let Some(t) = INNING_WIN_TICKER.captures(&ticker) else { return String::new() };
    let lado = lado.to_lowercase();
    if lado != "yes" && lado != "no" {
        return String::new();
    }
    let bruto = clean(titulo);
    let Some(corte) = bruto.find(" - ") else { return String::new() };
    let confronto = &bruto[..corte];
    let times: Vec<&str> = VS_SPLIT.split(confronto).collect();
    if times.len() != 2 {
        return String::new();
    }
    let detalhe = &bruto[corte + 3..];
    let titulo_mercado = mercado.and_then(|m| m.get("title")).and_then(Value::as_str).map(|t| WINS_FINAL.replace(t, "").into_owned());
    let sub = mercado.and_then(|m| m.get("yes_sub_title")).map(|v| clean_v(Some(v)));
    let do_detalhe = {
        let a = INNING_PREFIXO.replace(detalhe, "");
        let b = ML_FINAL.replace(&a, "");
        NO_FINAL.replace(&b, "").into_owned()
    };
    let candidatos = [titulo_mercado.unwrap_or_default(), sub.unwrap_or_default(), do_detalhe];
    let escolhidos: Vec<&str> = times
        .iter()
        .copied()
        .filter(|time| candidatos.iter().any(|c| so_alfanumerico_minusculo(c) == so_alfanumerico_minusculo(time)))
        .collect();
    // Nunca adivinha um time por abreviação nem inverte um No no adversário.
    if escolhidos.len() != 1 {
        return String::new();
    }
    format!("{confronto} - {} Inning {} ML{}", ordinal_texto(&t[1]), escolhidos[0], if lado == "no" { " No" } else { "" })
}

regex_exata!(DS_INCLUINDO, r"(?i)\b(?:including|excluding)\s+(?:overtime|extra time)\b");
regex_exata!(DS_ORDINAL, r"(?i)\b(first|second|third|fourth|fifth|sixth|seventh|eighth|ninth)\s+(quarter|period|set|map)\b");
regex_exata!(DS_NUMERADO, r"(?i)\b(\d+)(?:st|nd|rd|th)\s+(Period|Set|Map)\b|\b(Set|Map)\s+(\d+)\b");
regex_exata!(DS_EXTRA_INNINGS, r"(?i)\bextra innings\b");
regex_exata!(DS_PRORROGACAO, r"(?i)^(?:overtime|extra time)(?: winner)?$");
regex_exata!(DS_EXTRA, r"(?i)^extra");
regex_exata!(DS_JOGO, r"(?i)^(?:the\s+)?(?:game|match|full game|regulation(?: time)?)(?: winner)?\??$");

/// `drawScope()`: `Some("")` = jogo inteiro; `None` = escopo não reconhecido.
pub fn draw_scope(valor: &str) -> Option<String> {
    let sem_incluindo = DS_INCLUINDO.replace_all(&clean(valor), "").into_owned();
    let texto = DS_ORDINAL
        .replace_all(&sem_incluindo, |c: &regex::Captures<'_>| {
            let i = ORDINAIS.iter().position(|o| *o == c[1].to_lowercase()).map_or(0, |i| i + 1);
            format!("{} {}", ordinal(i as i64), &c[2])
        })
        .into_owned();
    let p = period_quarter(&texto);
    if !p.is_empty() {
        return Some(p);
    }
    if let Some(c) = DS_NUMERADO.captures(&texto) {
        let n: i64 = c.get(1).or(c.get(4)).map_or("0", |m| m.as_str()).parse().unwrap_or(0);
        let unidade = c.get(2).or(c.get(3)).map_or("", |m| m.as_str());
        if n > 0 {
            return Some(format!("{} {}", ordinal(n), capitalizar(unidade)));
        }
    }
    if DS_EXTRA_INNINGS.is_match(&texto) {
        return Some("Extra Innings".into());
    }
    if DS_PRORROGACAO.is_match(&texto) {
        return Some(if DS_EXTRA.is_match(&texto) { "Extra Time" } else { "Overtime" }.into());
    }
    if DS_JOGO.is_match(&texto) {
        return Some(String::new());
    }
    None
}

regex_exata!(LADO_EMPATE, r"(?i)^(Yes|No|Tie|Draw)$");
regex_exata!(NENHUM_VENCE, r"(?i)^(?:Will\s+)?neither team wins?(?:\s+(?:the\s+)?(.+?))?\??$");
regex_exata!(EMPATAM, r"(?i)^(?:Will\s+)?(?:the\s+)?(?:teams|both teams|game|match)\s+(?:tie(?:s)?(?:\s+scoring)?|draw|(?:end|finish)\s+in\s+(?:a\s+)?(?:tie|draw))(?:\s+(?:in|at|after)\s+(?:the\s+)?(.+?))?\??$");
regex_exata!(EMPATE_SIMPLES, r"(?i)^(?:Tie|Draw)\??$");
regex_exata!(SELECAO_EMPATE, r"(?i)^(?:Tie|Draw)$");
regex_exata!(VENCEDOR_RESULTADO, r"(?i)\b(?:winner|result)\b");
regex_exata!(APOS_INNING, r"(?i)\bafter\s+(?:the\s+)?\d+(?:st|nd|rd|th)\s+inning\b");
regex_exata!(E_NO, r"(?i)^No$");
regex_exata!(E_YES, r"(?i)^Yes$");

/// Último trecho depois de `:` (`q.split(':').at(-1)`).
fn depois_ultimo_dois_pontos(q: &str) -> String {
    clean(q.rsplit(':').next().unwrap_or(""))
}

/// `drawDetail()`.
pub fn draw_detail(pergunta: &str, resultado: &str, escopo: &str) -> String {
    let q = clean(pergunta);
    let cauda = depois_ultimo_dois_pontos(&q);
    let lado = clean(resultado);
    if !LADO_EMPATE.is_match(&lado) {
        return String::new();
    }
    let nenhum = NENHUM_VENCE.captures(&cauda);
    let empatam = EMPATAM.captures(&cauda);
    let simples = EMPATE_SIMPLES.is_match(&cauda);
    let selecionado = SELECAO_EMPATE.is_match(&lado) && VENCEDOR_RESULTADO.is_match(&q);
    if nenhum.is_none() && empatam.is_none() && !simples && !selecionado {
        return String::new();
    }
    // "after the 5th inning" pode ser acumulado, diferente de pontuar NA entrada.
    if APOS_INNING.is_match(&q) {
        return String::new();
    }
    let explicito_bruto = nenhum.as_ref().map(|c| grupo(c, 1)).filter(|s| !s.is_empty()).or_else(|| empatam.as_ref().map(|c| grupo(c, 1)));
    let explicito = clean(explicito_bruto.unwrap_or(""));
    let mut p = draw_scope(if explicito.is_empty() { &q } else { &explicito });
    if p.is_none() && explicito.is_empty() {
        p = draw_scope(escopo);
    }
    let Some(p) = p else { return String::new() };
    format!("{}Empate{}", if p.is_empty() { String::new() } else { format!("{p} ") }, if E_NO.is_match(&lado) { " No" } else { "" })
}

regex_exata!(SIM_NAO, r"(?i)^(yes|no)$");
regex_exata!(TIME_INVALIDO, r"(?i)^(?:Yes|No|Over|Under|Draw|Tie|Neither(?: team)?|None|No team)$");
regex_exata!(VS_DENTRO, r"(?i)\s+vs\.?\s+");
regex_exata!(NENHUM_RACE, r"(?i)^(?:Will\s+)?neither team\s+(?:reach|score)\s+(\d+)\s+points?(?:\s+first)?\??$");
regex_exata!(RACE, r"(?i)^(?:Will\s+)?(.+?)\s+(?:reach|score)\s+(\d+)\s+points?\s+first\??$");
regex_exata!(RACE_GENERICO, r"(?i)^(?:Race to|First to)\s+(\d+)\s+(?:points?|pontos)\??$");
regex_exata!(NENHUMA_SELECAO, r"(?i)^(?:Neither(?: team)?|None|No team)$");
regex_exata!(PRIMEIRO_TD, r"(?i)^(?:Will\s+)?(.+?)\s+scores?\s+(?:the\s+)?(?:first|1st)\s+(?:TD|touchdown)(?:\s+in\s+(?:the\s+)?(.+?))?\??$");
regex_exata!(TD_GENERICO, r"(?i)^(?:First Team to Score (?:a )?(?:TD|Touchdown)|First Touchdown Scorer)\??$");
regex_exata!(PRIMEIRAS_ENTRADAS, r"(?i)^(?:Will\s+)?(.+?)\s+(?:wins?\s+(?:the\s+)?)?(?:first|1st)\s+(\d+|five)\s+innings?(?:\s+winner)?\??$");
regex_exata!(CINCO, r"(?i)five");
regex_exata!(VENCE_SIMPLES, r"(?i)^(.+?)\s+wins?\??$");
regex_exata!(PRIMEIRAS_N, r"^1st \d+ Innings$");

fn time_valido(time: &str) -> bool {
    !time.is_empty() && !TIME_INVALIDO.is_match(time) && !VS_DENTRO.is_match(time)
}

fn numero_js(s: &str) -> f64 {
    js::number(Some(&Value::String(s.to_string())))
}

/// Inteiro `n` formatado como o JavaScript (`${n}`).
fn n_texto(n: f64) -> String {
    js::numero_texto(n).unwrap_or_else(|| "NaN".into())
}

/// `selectionDetail()`: Race, First TD, primeiras N entradas e empate.
pub fn selection_detail(pergunta: &str, resultado: &str, escopo: &str) -> String {
    selection_detail_escopo(pergunta, resultado, &Escopo::de_texto(escopo))
}

/// O `scope` de `kalshiDetail()`/`selectionDetail()` como cada trecho do JS o
/// lê: `[a, scope].join(' ')` (`null` vira vazio), `q + ' ' + scope` e
/// `String(x || scope)` (`null` vira "null") e `clean(scope)` (falso vira vazio).
/// `undefined` vira `''` pelo valor padrão do parâmetro.
pub struct Escopo {
    juncao: String,
    concatenado: String,
    limpo: String,
}

impl Escopo {
    pub fn de_texto(s: &str) -> Self {
        Escopo { juncao: s.to_string(), concatenado: s.to_string(), limpo: clean(s) }
    }

    pub fn de_valor(v: Option<&Value>) -> Self {
        match v {
            None => Escopo::de_texto(""),
            Some(Value::Null) => Escopo { juncao: String::new(), concatenado: "null".into(), limpo: String::new() },
            Some(v) => Escopo { juncao: js::texto(Some(v)), concatenado: js::texto(Some(v)), limpo: clean_v(Some(v)) },
        }
    }
}

fn selection_detail_escopo(pergunta: &str, resultado: &str, escopo: &Escopo) -> String {
    let q = clean(pergunta);
    let cauda = depois_ultimo_dois_pontos(&q);
    let selecionado = clean(resultado);
    let sim_nao = SIM_NAO.is_match(&selecionado);
    let negativo = if E_NO.is_match(&selecionado) { " No" } else { "" };
    let p = period_quarter(&format!("{q} {}", escopo.concatenado));
    let prefixo = if p.is_empty() { String::new() } else { format!("{p} ") };
    let escopo = escopo.limpo.as_str();
    let empate = draw_detail(&q, &selecionado, escopo);
    if !empate.is_empty() {
        return empate;
    }
    if let Some(c) = NENHUM_RACE.captures(&cauda) {
        let n = numero_js(&c[1]);
        if sim_nao && n > 0.0 {
            return format!("{prefixo}Qualquer equipe Race to {} pontos{}", n_texto(n), if E_YES.is_match(&selecionado) { " No" } else { "" });
        }
    }
    let race = RACE.captures(&cauda);
    let race_generico = RACE_GENERICO.captures(&cauda);
    if race.is_some() || race_generico.is_some() {
        let time = if sim_nao { clean(race.as_ref().map_or("", |c| grupo(c, 1))) } else { selecionado.clone() };
        let bruto_n = race.as_ref().map(|c| grupo(c, 2)).filter(|s| !s.is_empty()).or_else(|| race_generico.as_ref().map(|c| grupo(c, 1)));
        let n = bruto_n.map_or(f64::NAN, numero_js);
        if race_generico.is_some() && !sim_nao && NENHUMA_SELECAO.is_match(&selecionado) && n > 0.0 {
            return format!("{prefixo}Qualquer equipe Race to {} pontos No", n_texto(n));
        }
        let confere = match &race {
            None => true,
            Some(c) => sim_nao || time.to_lowercase() == clean(&c[1]).to_lowercase(),
        };
        if time_valido(&time) && n > 0.0 && confere {
            return format!("{prefixo}{time} Race to {} pontos{negativo}", n_texto(n));
        }
    }
    let primeiro_td = PRIMEIRO_TD.captures(&cauda);
    if primeiro_td.is_some() || TD_GENERICO.is_match(&cauda) {
        let time = if sim_nao { clean(primeiro_td.as_ref().map_or("", |c| grupo(c, 1))) } else { selecionado.clone() };
        let periodo_explicito = primeiro_td.as_ref().map(|c| grupo(c, 2)).filter(|s| !s.is_empty());
        let confere = match &primeiro_td {
            None => true,
            Some(c) => sim_nao || time.to_lowercase() == clean(&c[1]).to_lowercase(),
        };
        if time_valido(&time) && periodo_explicito.is_none_or(|p| !period(p).is_empty()) && confere {
            return format!("{prefixo}{time} First TD{negativo}");
        }
    }
    if sim_nao {
        if let Some(c) = PRIMEIRAS_ENTRADAS.captures(&cauda) {
            let time = clean(&c[1]);
            let n = if CINCO.is_match(&c[2]) { 5.0 } else { numero_js(&c[2]) };
            if time_valido(&time) && n > 0.0 {
                return format!("1st {} Innings {time} ML{negativo}", n_texto(n));
            }
        }
        if PRIMEIRAS_N.is_match(&p) {
            if let Some(c) = VENCE_SIMPLES.captures(&q) {
                if time_valido(&c[1]) {
                    return format!("{p} {} ML{negativo}", clean(&c[1]));
                }
            }
        }
    }
    String::new()
}

const ESCOPO: &str = r"(?:Full Game|Reg(?:ulation)? Time|(?:first|1st|second|2nd) Half|[1-4](?:st|nd|rd|th) Quarter|[1-4]Q|[12]H|(?:first|1st) (?:\d+|five) Innings?|\d+(?:st|nd|rd|th) (?:Inning|Period|Set|Map)|Overtime|Extra Time)";
const UNIDADES: &str = r"points?|goals?|runs?|corners?|corner kicks?|games?|sets?|rounds?|rebounds?|assists?|strikeouts?|steals?|blocks?|shots on target|three.pointers?";

static CT_PREFIXO: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| js::re_exata(&format!("(?i)^{ESCOPO}$")));
static CT_TOTAL: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    js::re_exata(&format!(
        r"(?i)^(?:(.+?)\s+(?:scores?\s+)?)?(over|under)\s+(\d+(?:[.,]\d+)?)\s+(?:({ESCOPO})\s+)?(?:total\s+)?({UNIDADES})(?:\s+(?:be\s+)?(?:scored|recorded))?(?:\s+in\s+(?:the\s+)?({ESCOPO}))?\??$"
    ))
});
regex_exata!(CT_WILL, r"(?i)^Will\s+(?:there be\s+)?");
regex_exata!(CT_TIME_INVALIDO, r"(?i)\b(?:if|and|or|not|no|neither|either|both|any|each|all|at least|first|race|win|reach|than|exactly|margin|consecutive|combined|total)\b|\bvs\.?\b|[?:]");
regex_exata!(CT_ROUNDS, r"^rounds?$");
regex_exata!(CT_OVER, r"(?i)^over$");
regex_exata!(CT_TRES_PONTOS, r"^three.pointers?$");

fn nome_escopo(s: &str) -> String {
    let p = period(s);
    if !p.is_empty() {
        return p;
    }
    draw_scope(s).unwrap_or_default()
}

/// `compactKalshiTotal()`: `[período] Over/Under linha unidade [No]`.
pub fn compact_kalshi_total(titulo: &str, lado: &str, escopo: &str) -> String {
    let texto = clean(titulo);
    let sn = clean(lado).to_lowercase();
    if sn != "yes" && sn != "no" {
        return String::new();
    }
    let dois_pontos = texto.rfind(':');
    let prefixo = dois_pontos.map_or("", |i| &texto[..i]);
    if !prefixo.is_empty() && !CT_PREFIXO.is_match(prefixo) {
        return String::new();
    }
    let pergunta_bruta = dois_pontos.map_or(texto.as_str(), |i| js::trim_js(&texto[i + 1..]));
    let pergunta = CT_WILL.replace(pergunta_bruta, "").into_owned();
    let Some(total) = CT_TOTAL.captures(&pergunta) else { return String::new() };
    let time = clean(grupo(&total, 1));
    if !time.is_empty() && CT_TIME_INVALIDO.is_match(&time) {
        return String::new();
    }
    let explicitos: Vec<String> =
        [prefixo, grupo(&total, 4), grupo(&total, 6)].iter().filter(|s| !s.is_empty()).map(|s| nome_escopo(s)).collect();
    let distintos: std::collections::HashSet<&String> = explicitos.iter().collect();
    if distintos.len() > 1 {
        return String::new();
    }
    let p = explicitos.first().cloned().unwrap_or_else(|| nome_escopo(escopo));
    let linha = grupo(&total, 3).replacen(',', ".", 1);
    let unidade = grupo(&total, 5).to_lowercase();
    let linha_n = numero_js(&linha);
    let inverso = sn == "no" && !(linha_n.is_finite() && linha_n.fract() == 0.0) && !CT_ROUNDS.is_match(&unidade);
    let ou = if inverso {
        if CT_OVER.is_match(grupo(&total, 2)) { "Under".to_string() } else { "Over".to_string() }
    } else {
        capitalizar(grupo(&total, 2))
    };
    let rotulo = match unit(&unidade, "") {
        "" => {
            let plural = |base: &str| unidade == base || unidade == format!("{base}s");
            if plural("game") {
                "games"
            } else if plural("set") {
                "sets"
            } else if plural("assist") {
                "assistências"
            } else if plural("strikeout") {
                "strikeouts"
            } else if plural("steal") {
                "roubos"
            } else if plural("block") {
                "bloqueios"
            } else if unidade == "shots on target" {
                "chutes no alvo"
            } else if CT_TRES_PONTOS.is_match(&unidade) {
                "cestas de 3"
            } else {
                ""
            }
        }
        u => u,
    };
    if rotulo.is_empty() {
        return String::new();
    }
    junta_nao_vazios(&[&time, &p, &ou, &linha, rotulo, if sn == "no" && !inverso { "No" } else { "" }])
}

regex_exata!(KD_AVANCA, r"(?i)^(.+?)\s+(?:to advance|advances?)\??$");
regex_exata!(KD_CONFRONTO_VENCEDOR, r"(?i)^(.+?)\s+vs\.?\s+(.+?)\s+Winner\??$");
regex_exata!(KD_ESPORTE, r"(?i)\b(?:soccer|football|basketball|baseball|hockey|tennis|game|match)\b");
regex_exata!(KD_RFI, r"(?i)^(?:First Inning Run\??|(?:Will\s+)?(?:either|any)\s+team\s+score\s+(?:a|at least one)\s+run\s+in\s+the\s+(?:first|1st)\s+inning\??)$");
regex_exata!(KD_NRFI, r"(?i)^(?:No (?:run|runs) in (?:the )?(?:first|1st) inning\??|(?:Will\s+)?neither team score (?:a )?run in the (?:first|1st) inning\??)$");
regex_exata!(KD_EXTRA, r"(?i)^(?:Will (?:the )?game go to |)(?:extra innings|Extra Innings)\??$");
regex_exata!(KD_VENCE_ENTRADA, r"(?i)^(?:Will\s+)?(.+?)\s+win(?:s)?(?:\s+the)?\s+(\d+(?:st|nd|rd|th)\s+inning|(?:first|second|third|fourth|fifth|sixth|seventh|eighth|ninth)\s+inning)\??$");
regex_exata!(KD_VENCE_PERIODO, r"(?i)^(?:Will\s+)?(.+?)\s+win(?:s)?(?:\s+the)?\s+(?:[1-4](?:st|nd|rd|th)\s+quarter|(?:first|second|1st|2nd)\s+half)\??$");
regex_exata!(KD_TOTAL, r"(?i)\b(over|under)\s+(\d+(?:[.,]\d+)?)\s+(?:(?:[1-4]Q|[12]H)\s+)?(goals?|runs?|points?)\b");
regex_exata!(KD_INN, r"(?i)Inn");
regex_exata!(KD_RUNS, r"(?i)^runs?$");
regex_exata!(KD_INTERROGACAO_FINAL, r"\?$");
regex_exata!(KD_ANTES_LIMPA, r"(?i)^(?:Will(?:\s+|$)|Reg(?:ulation)?\s*Time\s*:\s*)");
regex_exata!(KD_SCORE_FINAL, r"(?i)\s+(?:score|scores)\s*$");
regex_exata!(KD_DOIS_PONTOS_FINAL, r":\s*$");
regex_exata!(KD_ORDINAL_INICIO, r"(?i)^(?:first|1st|second|2nd|third|3rd|\d+)");
regex_exata!(KD_SO_ESCOPO, r"(?i)^(?:Full Game|First 5 innings|1st Half|2nd Half)\s*:?$");
regex_exata!(KD_THERE_BE, r"(?i)^(?:will there be|there be)$");

fn sem_acentos_alfanumerico(s: &str) -> String {
    clean(s)
        .nfd()
        .filter(|c| !('\u{0300}'..='\u{036f}').contains(c))
        .collect::<String>()
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        .collect()
}

/// `kalshiDetail(m, side, scope)` com `americanFootball = false`.
pub fn kalshi_detail(m: &Value, lado: Option<&str>, escopo: &str) -> String {
    kalshi_detail_js(m, lado.unwrap_or(""), &Escopo::de_texto(escopo), false)
}

regex_exata!(KD_VENCE_AMERICANO, r"(?i)^(?:Will\s+)?(.+?)\s+win(?:s)?(?:\s+the)?(?:\s+([1-4](?:st|nd|rd|th)\s+quarter|(?:first|second|1st|2nd)\s+half))?\??$");

/// `kalshiDetail(m, side, scope, americanFootball)`.
pub fn kalshi_detail_js(m: &Value, lado_txt: &str, escopo: &Escopo, americano: bool) -> String {
    if !js::truthy(Some(m)) {
        return String::new();
    }
    let lado = Some(lado_txt);
    let titulo = clean_v(ou(m.get("title"), m.get("subtitle")));
    let pergunta = match titulo.rfind(':') {
        Some(i) => js::trim_js(&titulo[i + 1..]).to_string(),
        None => titulo.clone(),
    };
    let subtitulo = clean_v(if lado == Some("no") { m.get("no_sub_title") } else { m.get("yes_sub_title") });
    let p = period(&[titulo.as_str(), escopo.juncao.as_str(), subtitulo.as_str()].join(" "));
    let sn = match lado {
        Some("yes") => "Yes",
        Some("no") => "No",
        _ => "",
    };
    let especial = selection_detail_escopo(&titulo, lado_txt, escopo);
    if !especial.is_empty() {
        return especial;
    }
    let compacto = compact_kalshi_total(&titulo, lado_txt, &escopo.limpo);
    if !compacto.is_empty() {
        return compacto;
    }
    if !sn.is_empty() {
        if let Some(c) = KD_AVANCA.captures(&pergunta) {
            return format!("{} Advances{}", clean(&c[1]), if lado == Some("no") { " No" } else { "" });
        }
        if let Some(c) = KD_CONFRONTO_VENCEDOR.captures(&pergunta) {
            let regras = m.get("rules_primary");
            let base = if js::truthy(regras) { js::texto(regras) } else { escopo.concatenado.clone() };
            if KD_ESPORTE.is_match(&base) {
                let escolhido = clean_v(m.get("yes_sub_title"));
                let norm_escolhido = sem_acentos_alfanumerico(&escolhido);
                let candidatos = [&c[1], &c[2]]
                    .iter()
                    .filter(|t| !norm_escolhido.is_empty() && sem_acentos_alfanumerico(t) == norm_escolhido)
                    .count();
                if candidatos == 1 {
                    return football_winner(&escolhido, &p, lado);
                }
            }
        }
        if americano {
            if let Some(c) = KD_VENCE_AMERICANO.captures(&pergunta) {
                let escopo_vitoria = c.get(2).map(|g| g.as_str()).filter(|s| !s.is_empty()).unwrap_or(&escopo.limpo);
                return football_winner(&c[1], escopo_vitoria, lado);
            }
        }
        // RFI é uma pergunta binária de corrida/sem corrida, não um total qualquer.
        if KD_RFI.is_match(&pergunta) {
            return format!("1st Inning {} 0.5 runs", if lado == Some("yes") { "Over" } else { "Under" });
        }
        if KD_NRFI.is_match(&pergunta) {
            return format!("1st Inning {} 0.5 runs", if lado == Some("yes") { "Under" } else { "Over" });
        }
        if KD_EXTRA.is_match(&pergunta) {
            return format!("Extra Innings {sn}");
        }
        if let Some(c) = KD_VENCE_ENTRADA.captures(&pergunta) {
            return format!("{} {} ML{}", period(&c[2]), clean(&c[1]), if lado == Some("no") { " No" } else { "" });
        }
    }
    if let Some(c) = KD_VENCE_PERIODO.captures(&pergunta) {
        if !p.is_empty() {
            if p == "1st Half" && !sn.is_empty() {
                return format!("{p} {} ML{}", clean(&c[1]), if lado == Some("no") { " No" } else { "" });
            }
            return format!("{} {p}{}", clean(&c[1]), if sn.is_empty() { String::new() } else { format!(" {sn}") });
        }
    }
    let mut total: Option<(regex::Captures<'_>, &str)> = None;
    for texto in [pergunta.as_str(), subtitulo.as_str()] {
        if let Some(c) = KD_TOTAL.captures(texto) {
            total = Some((c, texto));
            break;
        }
    }
    let Some((total, entrada)) = total else { return String::new() };
    // Um No binário não é um segundo Over: o contrato fica explícito.
    if lado == Some("no") && !(!p.is_empty() && KD_INN.is_match(&p) && KD_RUNS.is_match(&total[3])) {
        let prefixo = if !p.is_empty() && period(&pergunta).is_empty() { format!("{p} ") } else { String::new() };
        return format!("{prefixo}{}: No", KD_INTERROGACAO_FINAL.replace(&pergunta, ""));
    }
    let inicio = total.get(0).map_or(0, |m| m.start());
    let antes = {
        let a = clean(&entrada[..inicio]);
        let b = KD_ANTES_LIMPA.replace(&a, "");
        KD_SCORE_FINAL.replace(&b, "").into_owned()
    };
    let so_periodo = js::trim_js(&KD_DOIS_PONTOS_FINAL.replace(&antes, "")).to_string();
    let time = if !antes.is_empty()
        && !(!p.is_empty() && period(&so_periodo) == p && KD_ORDINAL_INICIO.is_match(&so_periodo))
        && !KD_SO_ESCOPO.is_match(&antes)
        && !KD_THERE_BE.is_match(&antes)
    {
        KD_DOIS_PONTOS_FINAL.replace(&antes, "").into_owned()
    } else {
        String::new()
    };
    let ou_txt = capitalizar(&total[1]);
    let linha = total[2].replacen(',', ".", 1);
    junta_nao_vazios(&[&time, &p, &ou_txt, &linha, unit(&total[3], ""), if lado == Some("no") { "No" } else { "" }])
}

regex_exata!(ET_TIPO_SEP, r"[\s-]+");
regex_exata!(ET_VS_PONTO, r"(?i)\s+vs\.\s+");
regex_exata!(ET_TIMES, r"(?i)^(.+?)\s+vs\s+(.+)$");
regex_exata!(ET_TIPO_DRAW, r"^(?:moneyline|(?:first_half|second_half|q[1-4]|(?:first|second|third|fourth)_quarter)_moneyline|soccer_halftime_result|rugby_halftime_result|rugby_second_half_result|baseball_team_first_five_winner|hockey_team_period[1-3]_winner)$");
regex_exata!(ET_QUARTO_TIPO, r"q([1-4])|^(first|second|third|fourth)_quarter");
regex_exata!(ET_PERIODO_TIPO, r"period([1-3])");
regex_exata!(ET_HOQUEI, r"^hockey_team_period([1-3])_winner$");
regex_exata!(ET_INNING, r"^baseball_team_inning([1-9]\d*)_winner$");
regex_exata!(ET_NOMEADO, r"(?i)^(?:Will\s+)?(.+?)\s+win(?:s)?\b");
regex_exata!(ET_OVER_UNDER_DRAW, r"(?i)^(Over|Under|Draw)$");
regex_exata!(ET_MAPA, r"(?i)\bMap\s+(\d+)\b|\b(\d+)(?:st|nd|rd|th)\s+Map\b");
regex_exata!(ET_SET, r"(?i)\bSet\s+(\d+)\b|\b(\d+)(?:st|nd|rd|th)\s+Set\b");
regex_exata!(ET_LINHA_OU, r"(?i)\b(?:O/U|Over/Under|Over|Under)\s+(\d+(?:[.,]\d+)?)");
regex_exata!(ET_LADO_OU, r"(?i)^(Over|Under)$");
regex_exata!(ET_OU_PERGUNTA, r"(?i)\bO/U|Over/Under");
regex_exata!(ET_METODO, r"(?i)^(?:Will\s+)?(.+?)\s+win\s+by\s+(knockout|KO/TKO|TKO|KO|submission|decision)\??$");
regex_exata!(ET_PODIO, r"(?i)^(?:Will\s+)?(.+?)\s+finish\s+on\s+the\s+podium\??$");

/// `extendedTitle()` para os mercados sintéticos que `kalshiSpecialTitle()`
/// monta: um único mercado `{slug, question, sportsMarketType}`.
fn extended_title_sintetico(pergunta: &str, tipo: &str, resultado: &str, titulo: &str, dica: &str) -> String {
    let tipo = ET_TIPO_SEP.replace_all(&clean(tipo).to_lowercase(), "_").into_owned();
    let q = clean(pergunta);
    let resultado = clean(resultado);
    if tipo.is_empty() || q.is_empty() || resultado.is_empty() {
        return String::new();
    }
    let fonte_confronto = [titulo, q.as_str()]
        .into_iter()
        .find(|v| VS_DENTRO.is_match(clean(v).split(" - ").next().unwrap_or("")))
        .map(str::to_string);
    let escolhido = if !dica.is_empty() { dica.to_string() } else { fonte_confronto.unwrap_or_else(|| titulo.to_string()) };
    let primeiro = clean(&escolhido).split(" - ").next().unwrap_or("").to_string();
    let mut base = ET_VS_PONTO.replace(&primeiro, " vs ").into_owned();
    if let Some(t) = ET_TIMES.captures(&base) {
        let a = js::trim_js(t[1].rsplit(':').next().unwrap_or("")).to_string();
        let b = js::trim_js(t[2].split(':').next().unwrap_or("")).to_string();
        base = format!("{a} vs {b}");
    }
    let prefixo = if base.is_empty() { String::new() } else { format!("{base} - ") };
    extended_core(&q, &tipo, &resultado, None, &prefixo)
}

/// `extendedTitle()` de um mercado exato `{question, line, sportsMarketType}`
/// sem confronto (o Predict.fun remove o prefixo depois).
pub fn extended_title_exato(pergunta: &str, tipo: &str, resultado: &str, linha: Option<&Value>) -> String {
    let tipo = ET_TIPO_SEP.replace_all(&clean(tipo).to_lowercase(), "_").into_owned();
    let q = clean(pergunta);
    let resultado = clean(resultado);
    if tipo.is_empty() || q.is_empty() || resultado.is_empty() {
        return String::new();
    }
    extended_core(&q, &tipo, &resultado, linha, "")
}

regex_exata!(ET_TIPO_RACE, r"_race_to_points$|_first_td(?:_team)?$|_first_touchdown(?:_scorer)?$");
regex_exata!(ET_RACE_LINHA, r"\bRace to (\d+) pontos");
regex_exata!(ET_TENIS_GAMES, r"^tennis_(?:(?:first|second|third|fourth|fifth)_)?set_games_totals$");
regex_exata!(ET_TENIS_SET_NOME, r"^tennis_(first|second|third|fourth|fifth)_set");
regex_exata!(ET_LINHA_DIRECAO, r"(?i)\b(O/U|Over/Under|Over|Under)\s+(\d+(?:[.,]\d+)?)");
regex_exata!(ET_AMBAS, r"^both_teams_to_score(?:_(?:first|second)_half)?$");
regex_exata!(ET_TORNEIO, r"(?i)\bwin\s+the\s+tournament\??$");
regex_exata!(ET_GOLFISTA, r"(?i)^(?:Will\s+)?(.+?)\s+win\s+the\s+tournament\??$");
regex_exata!(
    ET_LEGADO,
    r"^(?:moneyline|spreads?|totals?|team_totals(?:_home|_away)?|team_totals_(?:home|away)|game_team_totals|h2_team_totals|total_corners|total_goals|both_teams_to_score|rebounds|map_handicap|tennis_(?:set_handicap|game_handicap|set_totals|match_totals|first_set_totals)|baseball_(?:team_first_five_(?:spread|total|winner)|game_extra_innings)|nrfi|soccer_(?:team_to_advance|halftime_result|team_totals|home_team_totals|away_team_totals|team_total_corners|first_half_team_totals|second_half_team_totals|first_half_total_corners|second_half_total_corners)|(?:first_half|second_half|q[1-4])_(?:moneyline|spreads|totals|team_totals)|football_team_(?:first|second)_half_winner|football_team_(?:first|second|third|fourth)_quarter_winner|p[1-3]_totals)$"
);
regex_exata!(ET_LEGADO_QUARTO, r"^(?:to_advance|(?:first|second|third|fourth)_quarter_(?:moneyline|totals|spreads))$");

/// `m.line != null && Number(m.line) !== n`.
fn linha_diverge(linha: Option<&Value>, n: f64) -> bool {
    linha.is_some_and(|l| !l.is_null()) && js::number(linha) != n
}

/// Corpo do `extendedTitle()` depois de achar o mercado exato. `q`, `tipo` e
/// `resultado` já vêm limpos e não vazios.
fn extended_core(q: &str, tipo: &str, resultado: &str, linha: Option<&Value>, prefixo: &str) -> String {
    let q = q.to_string();
    let tipo = tipo.to_string();
    let resultado = resultado.to_string();
    let sn = if SIM_NAO.is_match(&resultado) { capitalizar(&resultado) } else { String::new() };
    let reserva = || format!("{prefixo}Mercado não confirmado: {q} — Seleção: {resultado}");
    if ET_TIPO_DRAW.is_match(&tipo) {
        let escopo_empate = if tipo.contains("first_half") || tipo.contains("halftime") {
            "1st Half".to_string()
        } else if tipo.contains("second_half") {
            "2nd Half".to_string()
        } else if tipo.contains("first_five") {
            "1st 5 Innings".to_string()
        } else if let Some(c) = ET_QUARTO_TIPO.captures(&tipo) {
            let n = c.get(1).map_or_else(
                || ["first", "second", "third", "fourth"].iter().position(|w| *w == grupo(&c, 2)).map_or(0, |i| i as i64 + 1),
                |m| m.as_str().parse().unwrap_or(0),
            );
            format!("{} Quarter", ordinal(n))
        } else if let Some(c) = ET_PERIODO_TIPO.captures(&tipo) {
            format!("{} Period", ordinal_texto(&c[1]))
        } else {
            "Game".to_string()
        };
        let empate = draw_detail(&q, &resultado, &escopo_empate);
        if !empate.is_empty() {
            return format!("{prefixo}{empate}");
        }
    }
    if ET_TIPO_RACE.is_match(&tipo) {
        let detalhe = selection_detail(&q, &resultado, "");
        if let Some(c) = ET_RACE_LINHA.captures(&detalhe) {
            if linha_diverge(linha, c[1].parse::<f64>().unwrap_or(f64::NAN)) {
                return reserva();
            }
        }
        return if detalhe.is_empty() { reserva() } else { format!("{prefixo}{detalhe}") };
    }
    if ET_TENIS_GAMES.is_match(&tipo) {
        let numero = ET_SET.captures(&q).and_then(|c| c.get(1).or(c.get(2)).map(|m| m.as_str().to_string()));
        let nomeado = ET_TENIS_SET_NOME
            .captures(&tipo)
            .map(|c| ["first", "second", "third", "fourth", "fifth"].iter().position(|w| *w == &c[1]).map_or(0, |i| i + 1))
            .unwrap_or(0);
        let n: f64 = match numero {
            Some(t) => t.parse().unwrap_or(f64::NAN),
            None => nomeado as f64,
        };
        let l = ET_LINHA_DIRECAO.captures(&q);
        let ou_txt = if ET_LADO_OU.is_match(&resultado) {
            resultado.clone()
        } else {
            match &l {
                Some(c) if !sn.is_empty() && ET_LADO_OU.is_match(&c[1]) => c[1].to_string(),
                _ => String::new(),
            }
        };
        let Some(l) = l else { return reserva() };
        let valor = l[2].replacen(',', ".", 1);
        if n == 0.0 || n.is_nan() || ou_txt.is_empty() || linha_diverge(linha, valor.parse::<f64>().unwrap_or(f64::NAN)) {
            return reserva();
        }
        return format!("{prefixo}{} Set {} {valor} games{}", ordinal(n as i64), capitalizar(&ou_txt), if sn == "No" { " No" } else { "" });
    }
    let vence = |escopo: &str| -> String {
        if escopo.is_empty() {
            return reserva();
        }
        let time = if sn.is_empty() {
            resultado.clone()
        } else {
            ET_NOMEADO.captures(&q).map_or(String::new(), |c| clean(&c[1]))
        };
        if !time.is_empty() && !ET_OVER_UNDER_DRAW.is_match(&time) {
            format!("{prefixo}{escopo} {time} ML{}", if sn == "No" { " No" } else { "" })
        } else {
            reserva()
        }
    };
    if let Some(c) = ET_INNING.captures(&tipo) {
        return vence(&format!("{} Inning", ordinal_texto(&c[1])));
    }
    if let Some(c) = ET_HOQUEI.captures(&tipo) {
        return vence(&format!("{} Period", ordinal_texto(&c[1])));
    }
    if ["tennis_set_winner", "tennis_first_set_winner", "volleyball_set_winner", "child_moneyline"].contains(&tipo.as_str()) {
        let mapa = tipo == "child_moneyline";
        let numero = if tipo == "tennis_first_set_winner" {
            Some("1".to_string())
        } else {
            let r: &regex::Regex = if mapa { &ET_MAPA } else { &ET_SET };
            r.captures(&q).and_then(|c| c.get(1).or(c.get(2)).map(|m| m.as_str().to_string()))
        };
        return vence(&numero.map_or(String::new(), |n| format!("{} {}", ordinal_texto(&n), if mapa { "Map" } else { "Set" })));
    }
    if ET_AMBAS.is_match(&tipo) {
        let p = if tipo.ends_with("second_half") {
            "2nd Half".to_string()
        } else if tipo.ends_with("first_half") {
            "1st Half".to_string()
        } else {
            period(&q)
        };
        return if sn.is_empty() {
            reserva()
        } else {
            format!("{prefixo}Both Teams to Score{} {sn}", if p.is_empty() { String::new() } else { format!(" {p}") })
        };
    }
    if tipo == "rugby_halftime_result" || tipo == "rugby_second_half_result" {
        return vence(if tipo == "rugby_halftime_result" { "1st Half" } else { "2nd Half" });
    }
    let props = match tipo.as_str() {
        "assists" => Some("Assistências"),
        "soccer_player_shots_on_target" => Some("Chutes no alvo"),
        "baseball_player_strikeouts" => Some("Strikeouts"),
        _ => None,
    };
    if let Some(nome_prop) = props {
        let jogador = clean(q.split(':').next().unwrap_or(""));
        let linha = ET_LINHA_OU.captures(&q);
        let lado_ou = ET_LADO_OU.captures(&resultado);
        if !q.contains(':') || jogador.is_empty() || linha.is_none() || (sn.is_empty() && lado_ou.is_none()) {
            return reserva();
        }
        let linha = linha.map(|c| c[1].replacen(',', ".", 1)).unwrap_or_default();
        let selecao = if sn.is_empty() {
            format!("{} {linha}", capitalizar(lado_ou.as_ref().map_or("", |c| grupo(c, 1))))
        } else {
            format!("O/U {linha} {sn}")
        };
        // Yes/No numa pergunta direcional não é Yes/No numa proposição O/U.
        if !sn.is_empty() && !ET_OU_PERGUNTA.is_match(&q) {
            return reserva();
        }
        return format!("{prefixo}{jogador} {nome_prop} {selecao}");
    }
    if ["cricket_first_inning_runs", "cricket_second_inning_runs", "first_innings_totals", "second_innings_totals"].contains(&tipo.as_str()) {
        let escopo = if tipo.contains("second") { "2nd Innings" } else { "1st Innings" };
        return match (ET_LINHA_OU.captures(&q), ET_LADO_OU.captures(&resultado)) {
            (Some(l), Some(o)) => format!("{prefixo}{escopo} {} {} runs", capitalizar(&o[1]), l[1].replacen(',', ".", 1)),
            _ => reserva(),
        };
    }
    if ["ufc_method_of_victory", "ufc_method_of_finish", "combat_method_of_victory"].contains(&tipo.as_str()) {
        return match ET_METODO.captures(&q) {
            Some(c) if !sn.is_empty() => {
                let nome = match c[2].to_lowercase().as_str() {
                    "knockout" => "nocaute",
                    "ko" => "KO",
                    "tko" => "TKO",
                    "ko/tko" => "KO/TKO",
                    "submission" => "finalização",
                    _ => "decisão",
                };
                format!("{prefixo}{} Vitória por {nome} {sn}", clean(&c[1]))
            }
            _ => reserva(),
        };
    }
    if tipo == "f1_race_podium" {
        return match ET_PODIO.captures(&q) {
            Some(c) if !sn.is_empty() => format!("{prefixo}{} Pódio {sn}", clean(&c[1])),
            _ => reserva(),
        };
    }
    if tipo == "moneyline" && ET_TORNEIO.is_match(&q) {
        return match ET_GOLFISTA.captures(&q) {
            Some(c) if !sn.is_empty() => format!("{prefixo}Torneio {} ML{}", clean(&c[1]), if sn == "No" { " No" } else { "" }),
            _ => reserva(),
        };
    }
    // Tipos estruturados desconhecidos nunca caem nos palpites genéricos.
    if ET_LEGADO.is_match(&tipo) || ET_LEGADO_QUARTO.is_match(&tipo) { String::new() } else { reserva() }
}

/// `sportUnit()`.
pub fn sport_unit(esporte: &str) -> &'static str {
    match clean(esporte).to_lowercase().as_str() {
        "basquete" | "futebol americano" | "basketball" | "american football" => "pontos",
        "beisebol" | "baseball" => "runs",
        "futebol" | "hóquei" | "soccer" | "hockey" => "gols",
        _ => "",
    }
}

regex_exata!(KS_OFICIAL, r"(?i)\b(?:in|of)\s+(?:the\s+)?(.+?\s+vs\.?\s+.+?)\s+(?:Pro Football|college football|professional baseball|Pro Basketball|college basketball)\s+game\b");
regex_exata!(KS_INTERROGACAO_DOIS_PONTOS, r"[?:]");
regex_exata!(KS_PERIODO_NUM, r"(?i)\b([1-3])(?:st|nd|rd)\s+period\b");
regex_exata!(KS_VENCE, r"(?i)\bwins?\b");
regex_exata!(KS_SET, r"(?i)\bwins?\b.*\b(?:Set\s+\d+|\d+(?:st|nd|rd|th)\s+Set)\b");
regex_exata!(KS_MAPA, r"(?i)\bwins?\b.*\b(?:Map\s+\d+|\d+(?:st|nd|rd|th)\s+Map)\b");
regex_exata!(KS_METODO, r"(?i)\bwin by (?:knockout|KO/TKO|TKO|KO|submission|decision)\??$");
regex_exata!(KS_PODIO, r"(?i)\bfinish on the podium\??$");
regex_exata!(KS_LINHA, r"(?i)\b(?:O/U|Over/Under|Over|Under)\s+\d");
regex_exata!(KS_CHUTES, r"(?i)\bshots on target\b");
regex_exata!(KS_STRIKEOUTS, r"(?i)\bstrikeouts\b");
regex_exata!(KS_ASSISTS, r"(?i)\bassists\b");

/// Entrada de `kalshiSpecialTitle()` montada por `kalshiTitle()`.
pub struct TituloEspecial<'a> {
    pub ticker: &'a str,
    pub lado: Option<&'a str>,
    pub titulo: &'a str,
    pub mercado: &'a Value,
    pub escopo: &'a str,
}

/// `kalshiSpecialTitle()`: totais compactos, Race/First TD, períodos de hóquei,
/// sets/mapas, método de vitória, pódio e props de jogador.
pub fn kalshi_special_title(d: &TituloEspecial<'_>) -> String {
    if !js::truthy(Some(d.mercado)) {
        return String::new();
    }
    let q = clean_v(d.mercado.get("title"));
    let escopo = clean(d.escopo);
    let fonte = format!("{q} {escopo}");
    let lado = d.lado.unwrap_or("");
    let mut detalhe = compact_kalshi_total(&q, lado, &escopo);
    if detalhe.is_empty() {
        detalhe = selection_detail(&q, lado, &escopo);
    }
    if !detalhe.is_empty() {
        let bruto = clean(d.titulo).split(" - ").next().unwrap_or("").to_string();
        let regras = clean_v(d.mercado.get("rules_primary"));
        let oficial = KS_OFICIAL.captures(&regras).map(|c| clean(&c[1])).unwrap_or_default();
        // `_titleDisplay` e `matchup` não existem na linha sintética.
        let confronto = ["", "", oficial.as_str(), bruto.as_str()]
            .into_iter()
            .find(|t| VS_DENTRO.is_match(t) && !KS_INTERROGACAO_DOIS_PONTOS.is_match(t));
        if let Some(c) = confronto {
            return format!("{} - {detalhe}", ET_VS_PONTO.replace(c, " vs "));
        }
    }
    let tipo = tipo_especial_kalshi(&q, &fonte);
    if tipo.is_empty() {
        return String::new();
    }
    let confronto = clean(d.titulo).split(" - ").next().unwrap_or("").to_string();
    let _ = d.ticker;
    extended_title_sintetico(&q, &tipo, lado, d.titulo, &confronto)
}

regex_exata!(MP_PREFIXO, r"(?i)^(?:the\s+)?(?:first|second|third|fourth|fifth|last|final|\d+(?:st|nd|rd|th))\s+(?:\d+\s+)?(?:half|halves|quarters?|periods?|innings?|sets?|rounds?)\b\s*((?:\([^()]+\)\s*)*)(?:of|in|for)\s+(?:the\s+)?");
regex_exata!(MP_CONFRONTO, r"(?i)\S\s+(?:vs\.?|versus|@)\s+\S");

/// `kalshiMatchupParts()`: separa "1st quarter of the" do confronto.
pub fn kalshi_matchup_parts(titulo: &str) -> (String, String) {
    let texto = clean(titulo);
    match MP_PREFIXO.captures(&texto) {
        Some(c) => {
            let fim = c.get(0).map_or(0, |m| m.end());
            let candidato = &texto[fim..];
            if MP_CONFRONTO.is_match(candidato.split(" - ").next().unwrap_or("")) {
                (candidato.to_string(), js::trim_js(grupo(&c, 1)).to_string())
            } else {
                (texto.clone(), String::new())
            }
        }
        None => (texto.clone(), String::new()),
    }
}

pub fn clean_kalshi_matchup(titulo: &str) -> String {
    kalshi_matchup_parts(titulo).0
}

regex_exata!(BQ_NO, r"(?i)\s+No$");
regex_exata!(BQ_NO_CAPTURA, r"(?i)\s+(No)$");

/// `kalshiBetQualifier()`: leva qualificadores como "(excluding overtime)" para a aposta.
pub fn kalshi_bet_qualifier(aposta: &str, fontes: &[&str]) -> String {
    let mut aposta = aposta.to_string();
    for fonte in fontes {
        let (_, qualificador) = kalshi_matchup_parts(fonte);
        if qualificador.is_empty() || aposta.to_lowercase().contains(&qualificador.to_lowercase()) {
            continue;
        }
        aposta = if BQ_NO.is_match(&aposta) {
            BQ_NO_CAPTURA
                .replace(&aposta, |c: &regex::Captures<'_>| format!(" {qualificador} {}", &c[1]))
                .into_owned()
        } else {
            format!("{aposta} {qualificador}")
        };
    }
    aposta
}

regex_exata!(MM_VS, r"(?i)\s+(?:vs\.?|versus|@)\s+");
regex_exata!(MM_VS_SIMPLES, r"(?i)\s+vs\.?\s+");
regex_exata!(MM_PERGUNTAS, r"(?i)\b(?:reach|score)\s+\d+\s+points?\s+first\??$|\bscores?\s+(?:the\s+)?first\s+(?:TD|touchdown)\??$|\b(?:first|1st)\s+\d+\s+innings?\s+winner\??$|\bneither team\s+(?:wins?|reach)\b");
regex_exata!(MM_TOTAL, r"(?i)^(?:Over|Under)\s+\d+(?:[.,]\d+)?\s+(?:gols|goals?|runs?|pontos|points?)(?:\s+No)?$");
regex_exata!(MM_VENCE, r"(?i)^.+?\s+wins\??$");

/// `kalshiTitleMissingMatchup()`.
pub fn kalshi_title_missing_matchup(linha: &Value, titulo: Option<&str>) -> bool {
    let plataforma = linha.get("_platform");
    if !(js::truthy(plataforma) && js::texto(plataforma).to_lowercase() == "kalshi") {
        return false;
    }
    let texto = match titulo {
        Some(t) => clean(t),
        None => clean_v(linha.get("title")),
    };
    if MM_VS.is_match(&texto) {
        return false;
    }
    let confronto = clean_v(linha.get("_kalshiTitleMeta").and_then(|m| m.get("matchup")));
    if MM_VS_SIMPLES.is_match(&confronto) {
        return true;
    }
    if MM_PERGUNTAS.is_match(&texto) {
        return true;
    }
    js::truthy(linha.get("sport")) && (MM_TOTAL.is_match(&texto) || MM_VENCE.is_match(&texto))
}

/// `exactMarket(d)` sobre `serde_json::Value`. `Err` quando o JS lançaria exceção.
pub fn exact_market<'a>(d: &Value, meta: Option<&'a Value>) -> Result<Option<&'a Value>, ()> {
    let mercados: &[Value] = meta.and_then(|m| m.get("markets")).and_then(Value::as_array).map_or(&[], |a| a.as_slice());
    let sem_aspas = |s: String| -> String {
        let s = s.strip_prefix('"').map(str::to_string).unwrap_or(s);
        s.strip_suffix('"').map(str::to_string).unwrap_or(s)
    };
    let asset = sem_aspas(clean_v(ou(ou(d.get("asset"), d.get("assetId")), d.get("tokenId"))));
    let condicao = clean_v(ou(d.get("conditionId"), d.get("condition_id"))).to_lowercase();
    let slug = clean_v(d.get("slug")).to_lowercase();
    let unico = |achados: Vec<&'a Value>| if achados.len() == 1 { Some(achados[0]) } else { None };
    if !asset.is_empty() {
        let mut achados = Vec::new();
        for m in mercados {
            if m.is_null() {
                return Err(());
            }
            let ids: &[Value] = match m.get("tokenIds") {
                Some(Value::Array(a)) => a,
                Some(v) if js::truthy(Some(v)) => return Err(()),
                _ => &[],
            };
            if ids.iter().any(|t| sem_aspas(clean_v(Some(t))) == asset) {
                achados.push(m);
            }
        }
        if let Some(m) = unico(achados) {
            return Ok(Some(m));
        }
    }
    if !condicao.is_empty() {
        let mut achados = Vec::new();
        for m in mercados {
            if m.is_null() {
                return Err(());
            }
            if clean_v(ou(m.get("conditionId"), m.get("condition_id"))).to_lowercase() == condicao {
                achados.push(m);
            }
        }
        if let Some(m) = unico(achados) {
            return Ok(Some(m));
        }
    }
    if !slug.is_empty() {
        let mut achados = Vec::new();
        for m in mercados {
            if m.is_null() {
                return Err(());
            }
            if clean_v(m.get("slug")).to_lowercase() == slug {
                achados.push(m);
            }
        }
        if let Some(m) = unico(achados) {
            return Ok(Some(m));
        }
    }
    Ok(None)
}

/// `String(d?._platform || padrao).toLowerCase()`.
pub fn plataforma(d: &Value, padrao: &str) -> String {
    let p = d.get("_platform");
    if js::truthy(p) { js::texto(p).to_lowercase() } else { padrao.to_string() }
}

/// Itens de `...(x || [])`: lista, texto (letra a letra) ou exceção.
pub fn espalhar(x: Option<&Value>) -> Result<Vec<Value>, ()> {
    match x {
        Some(Value::Array(a)) => Ok(a.clone()),
        Some(Value::String(s)) => Ok(s.chars().map(|c| Value::String(c.to_string())).collect()),
        Some(v) if js::truthy(Some(v)) => Err(()),
        _ => Ok(Vec::new()),
    }
}

regex_exata!(DT_EMPATE, r"(?i)^(?:Will\s+)?(.+?)\s+vs\.?\s+(.+?)\s+(?:end|finish)\s+in\s+a\s+draw(?:\s+(?:in|at)\s+(?:the\s+)?(.+?))?\??$");

/// `drawTitle(d)`. `Err` quando o JS lançaria exceção.
pub fn draw_title(d: &Value) -> Result<String, ()> {
    if plataforma(d, "polymarket") != "polymarket" {
        return Ok(String::new());
    }
    let proprio = exact_market(d, d.get("_sportMeta"))?;
    let pergunta = clean_v(ou(proprio.and_then(|m| m.get("question")), d.get("title")));
    let lado = clean_v(d.get("outcome"));
    if !SIM_NAO.is_match(&lado) {
        return Ok(String::new());
    }
    let Some(c) = DT_EMPATE.captures(&pergunta) else { return Ok(String::new()) };
    let tipo = clean_v(proprio.and_then(|m| m.get("sportsMarketType"))).to_lowercase();
    let explicito = grupo(&c, 3);
    let escopo = draw_scope(explicito).filter(|s| !s.is_empty()).unwrap_or_else(|| {
        if tipo.contains("first_half") || tipo.contains("halftime") {
            "1st Half".into()
        } else if tipo.contains("second_half") {
            "2nd Half".into()
        } else {
            String::new()
        }
    });
    if !explicito.is_empty() && escopo.is_empty() {
        return Ok(String::new());
    }
    Ok(format!(
        "{} vs {} - Empate{}{}",
        clean(&c[1]),
        clean(&c[2]),
        if escopo.is_empty() { String::new() } else { format!(" {escopo}") },
        if E_NO.is_match(&lado) { " No" } else { "" }
    ))
}

regex_exata!(CB_TOTAL, r"(?i)\b(Over|Under|O\/U)\s+(\d+(?:[.,]\d+)?)\s+rounds?\b");
regex_exata!(CB_LUTA, r"(?i)^(?:UFC|Bellator|PFL|Boxing|Boxe)[^:]*:\s*");
regex_exata!(CB_HIFEN_OU, r"(?i)\s+-\s+(?:Over|Under|O\/U)\b.*$");
regex_exata!(CB_DOIS_PONTOS_OU, r"(?i):\s*(?:Over|Under|O\/U)\b.*$");

/// `combatTitle(d)`: total de rounds de lutas. `Err` = exceção no JS.
pub fn combat_title(d: &Value) -> Result<String, ()> {
    let Some(m) = exact_market(d, d.get("_sportMeta"))? else { return Ok(String::new()) };
    let q = clean_v(m.get("question"));
    let Some(total) = CB_TOTAL.captures(&q) else { return Ok(String::new()) };
    let resultado = clean_v(d.get("outcome"));
    let mut lado = if ET_LADO_OU.is_match(&resultado) { capitalizar(&resultado) } else { String::new() };
    let mut negativo = "";
    if lado.is_empty() && SIM_NAO.is_match(&resultado) && ET_LADO_OU.is_match(&total[1]) {
        lado = capitalizar(&total[1]);
        negativo = if E_NO.is_match(&resultado) { " No" } else { "" };
    }
    if lado.is_empty() {
        return Ok(String::new());
    }
    let mut candidatos = vec![clean_v(d.get("title"))];
    candidatos.extend(espalhar(d.get("_sportMeta").and_then(|s| s.get("texts")))?.iter().map(|t| clean_v(Some(t))));
    let Some(base) = candidatos.into_iter().find(|t| VS_DENTRO.is_match(t)) else { return Ok(String::new()) };
    let a = CB_LUTA.replace(&base, "");
    let b = CB_HIFEN_OU.replace(&a, "");
    let c = CB_DOIS_PONTOS_OU.replace(&b, "");
    let confronto = VS_DENTRO.replace(&c, " vs ");
    Ok(format!("{confronto} - {lado} {} rounds{negativo}", total[2].replacen(',', ".", 1)))
}

/// `extendedTitle(d, matchupHint)`. `Err` = exceção no JS.
pub fn extended_title(d: &Value, dica: &str) -> Result<String, ()> {
    Ok(match exact_market(d, d.get("_sportMeta"))? {
        Some(m) => extended_title_mercado(d, m, dica),
        None => String::new(),
    })
}

/// `extendedTitle()` depois de achar o mercado exato `m` da linha `d`.
fn extended_title_mercado(d: &Value, m: &Value, dica: &str) -> String {
    let tipo = ET_TIPO_SEP.replace_all(&clean_v(m.get("sportsMarketType")).to_lowercase(), "_").into_owned();
    let q = clean_v(m.get("question"));
    let resultado = clean_v(d.get("outcome"));
    if tipo.is_empty() || q.is_empty() || resultado.is_empty() {
        return String::new();
    }
    let pergunta = Value::String(q.clone());
    let fonte = [d.get("_titleDisplay"), d.get("title"), Some(&pergunta)]
        .into_iter()
        .find(|v| VS_DENTRO.is_match(clean_v(*v).split(" - ").next().unwrap_or("")))
        .flatten();
    let escolhido = if !dica.is_empty() {
        clean(dica)
    } else if let Some(v) = fonte {
        clean_v(Some(v))
    } else {
        clean_v(ou(d.get("_titleDisplay"), d.get("title")))
    };
    let mut base = ET_VS_PONTO.replace(escolhido.split(" - ").next().unwrap_or(""), " vs ").into_owned();
    if let Some(t) = ET_TIMES.captures(&base) {
        let a = js::trim_js(t[1].rsplit(':').next().unwrap_or("")).to_string();
        let b = js::trim_js(t[2].split(':').next().unwrap_or("")).to_string();
        base = format!("{a} vs {b}");
    }
    let prefixo = if base.is_empty() { String::new() } else { format!("{base} - ") };
    extended_core(&q, &tipo, &resultado, m.get("line"), &prefixo)
}

/// `kalshiSpecialTitle(d)` sobre a linha inteira (com `_titleDisplay` e o
/// `matchup` do metadado).
pub fn kalshi_special_title_linha(d: &Value) -> String {
    let meta = d.get("_kalshiTitleMeta");
    let Some(mercado) = meta.and_then(|m| m.get("market")).filter(|m| js::truthy(Some(m))) else { return String::new() };
    if plataforma(d, "") != "kalshi" {
        return String::new();
    }
    let q = clean_v(mercado.get("title"));
    let escopo = clean_v(meta.and_then(|m| m.get("scope")));
    let fonte = format!("{q} {escopo}");
    let lado = clean_v(d.get("outcome"));
    let mut detalhe = compact_kalshi_total(&q, &lado, &escopo);
    if detalhe.is_empty() {
        detalhe = selection_detail(&q, &lado, &escopo);
    }
    if !detalhe.is_empty() {
        let bruto = clean_v(d.get("title")).split(" - ").next().unwrap_or("").to_string();
        let regras = clean_v(mercado.get("rules_primary"));
        let oficial = KS_OFICIAL.captures(&regras).map(|c| clean(&c[1])).unwrap_or_default();
        let confirmado = clean_v(d.get("_titleDisplay")).split(" - ").next().unwrap_or("").to_string();
        let do_meta = clean_v(meta.and_then(|m| m.get("matchup")));
        let confronto = [confirmado.as_str(), do_meta.as_str(), oficial.as_str(), bruto.as_str()]
            .into_iter()
            .find(|t| VS_DENTRO.is_match(t) && !KS_INTERROGACAO_DOIS_PONTOS.is_match(t));
        if let Some(c) = confronto {
            return format!("{} - {detalhe}", ET_VS_PONTO.replace(c, " vs "));
        }
    }
    let tipo = tipo_especial_kalshi(&q, &fonte);
    if tipo.is_empty() {
        return String::new();
    }
    let confronto = clean_v(d.get("title")).split(" - ").next().unwrap_or("").to_string();
    // `exactMarket()` acha o mercado sintético pelo slug `d.slug || 'exact'`
    // (ele não tem tokenIds nem conditionId), a não ser que o slug fique vazio.
    let exato = Value::String("exact".into());
    let slug = ou(d.get("slug"), Some(&exato)).cloned().unwrap_or(Value::Null);
    if clean_v(Some(&slug)).is_empty() {
        return String::new();
    }
    let sintetico = serde_json::json!({ "slug": slug, "question": q, "sportsMarketType": tipo });
    extended_title_mercado(d, &sintetico, &confronto)
}

/// O `type` que `kalshiSpecialTitle()` deduz do título do mercado.
fn tipo_especial_kalshi(q: &str, fonte: &str) -> String {
    if let Some(c) = KS_PERIODO_NUM.captures(fonte).filter(|_| KS_VENCE.is_match(q)) {
        format!("hockey_team_period{}_winner", &c[1])
    } else if KS_SET.is_match(q) {
        "tennis_set_winner".into()
    } else if KS_MAPA.is_match(q) {
        "child_moneyline".into()
    } else if KS_METODO.is_match(q) {
        "combat_method_of_victory".into()
    } else if KS_PODIO.is_match(q) {
        "f1_race_podium".into()
    } else if q.contains(':') && KS_LINHA.is_match(q) {
        if KS_CHUTES.is_match(q) {
            "soccer_player_shots_on_target".into()
        } else if KS_STRIKEOUTS.is_match(q) {
            "baseball_player_strikeouts".into()
        } else if KS_ASSISTS.is_match(q) {
            "assists".into()
        } else {
            String::new()
        }
    } else {
        String::new()
    }
}

/// `inningWinnerTitle(row)` sobre a linha inteira. `Err` quando o JS lançaria
/// exceção (`market.title` que não é texto).
pub fn inning_winner_title_linha(d: &Value) -> Result<String, ()> {
    if plataforma(d, "") != "kalshi" {
        return Ok(String::new());
    }
    let ticker = js::texto_ou_vazio(ou(ou(d.get("slug"), d.get("ticker")), d.get("market_ticker"))).to_uppercase();
    let Some(t) = INNING_WIN_TICKER.captures(&ticker) else { return Ok(String::new()) };
    let lado = js::texto_ou_vazio(d.get("outcome")).to_lowercase();
    if lado != "yes" && lado != "no" {
        return Ok(String::new());
    }
    let bruto = clean_v(d.get("title"));
    let Some(corte) = bruto.find(" - ") else { return Ok(String::new()) };
    let confronto = &bruto[..corte];
    let times: Vec<&str> = VS_SPLIT.split(confronto).collect();
    if times.len() != 2 {
        return Ok(String::new());
    }
    let detalhe = &bruto[corte + 3..];
    let mercado = d.get("_kalshiTitleMeta").and_then(|m| m.get("market"));
    let titulo_mercado = match mercado.and_then(|m| m.get("title")) {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => WINS_FINAL.replace(s, "").into_owned(),
        Some(_) => return Err(()),
    };
    let do_detalhe = {
        let a = INNING_PREFIXO.replace(detalhe, "");
        let b = ML_FINAL.replace(&a, "");
        NO_FINAL.replace(&b, "").into_owned()
    };
    let candidatos = [titulo_mercado, clean_v(mercado.and_then(|m| m.get("yes_sub_title"))), do_detalhe];
    let escolhidos: Vec<&str> = times
        .iter()
        .copied()
        .filter(|time| candidatos.iter().any(|c| so_alfanumerico_minusculo(c) == so_alfanumerico_minusculo(time)))
        .collect();
    if escolhidos.len() != 1 {
        return Ok(String::new());
    }
    Ok(format!("{confronto} - {} Inning {} ML{}", ordinal_texto(&t[1]), escolhidos[0], if lado == "no" { " No" } else { "" }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exemplos() {
        assert_eq!(period("Lakers 2nd quarter"), "2º quarto");
        assert_eq!(period("first 5 innings"), "1st 5 Innings");
        assert_eq!(compact_kalshi_total("Full Game: Over 228.5 points scored", "no", ""), "Full Game Under 228.5 pontos".replace("Full Game ", ""));
        assert_eq!(clean_kalshi_matchup("1st quarter of the Lakers vs Celtics"), "Lakers vs Celtics");
        assert_eq!(kalshi_bet_qualifier("Lakers ML No", &["1st half (excluding overtime) of Lakers vs Celtics"]), "Lakers ML (excluding overtime) No");
    }
}
