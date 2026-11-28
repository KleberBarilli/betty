//! Título de exibição de uma aposta, igual a `tituloExibicao()` do
//! `dashboard.js`, com `normalizarNomeMercadoPolymarket()` (e os auxiliares
//! internos dela: `cleanTeam`, `stripMarketSuffix`, `matchupFrom`,
//! `findMatchup`, `normalizeUnit`, `extractLine`, `extractOU`,
//! `extractPeriod`, `findSpecificTeam`, `findBaseballTeam`...) e
//! `normalizarNomeMercadoKalshi()`. As funções de `market-title-utils.js` vêm
//! de `titles.rs`.
//!
//! Parte da sincronização no Rust (`docs/desempenho.md`): ainda não é usada
//! pelas rotas, e o oráculo (`tests::oraculo_titulos_exibicao`) compara com o
//! JS real.
//!
//! - O `_tituloExibicaoCache` do JS não existe aqui: com o mesmo título,
//!   resultado, plataforma e id e sem metadado, o JS devolve o título de outra
//!   linha. A função pura é o comportamento sem cache.
//! - Onde o JS lança exceção fora do `try` de `tituloExibicao()` (mercado nulo
//!   em `_sportMeta.markets`, `market.title` que não é texto num ticker
//!   `INNINGWIN`...), o resultado é `Err`. Dentro do `try`, a exceção vira
//!   `d.title || ""`, como no JS.
//! - `indexOf`/`slice`/`length` do JS contam unidades UTF-16.

use std::cell::RefCell;
use std::collections::HashMap;

use regex::Regex;
use serde_json::{Value, json};
use unicode_normalization::UnicodeNormalization;

use crate::categorias::{self, Excecao};
use crate::titles::{self, Escopo, clean, clean_v, ou};
use crate::{js, regex_exata};

type R<T> = Result<T, Excecao>;

fn exc<T>(r: Result<T, ()>) -> R<T> {
    r.map_err(|()| Excecao)
}

fn texto(s: impl Into<String>) -> Value {
    Value::String(s.into())
}

/// `d?.title || ""`.
fn titulo_ou_vazio(d: &Value) -> Value {
    d.get("title").filter(|t| js::truthy(Some(t))).cloned().unwrap_or_else(|| texto(""))
}

/// `String(v || "").trim()`.
fn aparado(v: Option<&Value>) -> String {
    js::trim_js(&js::texto_ou_vazio(v)).to_string()
}

fn trim(s: &str) -> String {
    js::trim_js(s).to_string()
}

/// Primeira letra maiúscula e o resto minúsculo (`x[0].toUpperCase() + x.slice(1).toLowerCase()`).
fn cap(s: &str) -> String {
    titles::capitalizar(s)
}

/// `/^"|"$/g` trocado por vazio.
fn sem_aspas(s: &str) -> &str {
    let s = s.strip_prefix('"').unwrap_or(s);
    s.strip_suffix('"').unwrap_or(s)
}

/// `a.toLowerCase().indexOf(b)` em unidades UTF-16.
fn indice_utf16(onde: &str, o_que: &str) -> Option<usize> {
    onde.find(o_que).map(|i| js::tamanho_utf16(&onde[..i]))
}

/// `classificarAposta(d) === nome`.
fn categoria_e(d: &Value, nome: &str) -> R<bool> {
    Ok(categorias::classificar(d)?.as_str() == Some(nome))
}

/// `sportUnit(classificarAposta(d))`.
fn unidade_da_categoria(d: &Value) -> R<&'static str> {
    Ok(titles::sport_unit(&clean_v(Some(&categorias::classificar(d)?))))
}

regex_exata!(ESPACOS, r"\s+");
regex_exata!(DOIS_PONTOS, r":\s*");

/// `s.split(/:\s*/).pop().trim()`.
fn depois_dos_dois_pontos(s: &str) -> String {
    trim(DOIS_PONTOS.split(s).last().unwrap_or(""))
}

regex_exata!(CT_INICIO, r"^[:\-–—]+\s*");
regex_exata!(CT_FIM, r"\s*[:\-–—]+\s*$");
regex_exata!(CT_MAIS, r"(?i)^More\s+Markets\s*[:\-–—]\s*");

/// `cleanTeam(v)`.
fn clean_team(v: &str) -> String {
    let a = ESPACOS.replace_all(v, " ");
    let b = CT_INICIO.replace(&a, "");
    let c = CT_FIM.replace(&b, "");
    trim(&CT_MAIS.replace(&c, ""))
}

regex_exata!(SM_1, r"(?i)\s*[-–—]\s*(?:More\s+Markets\s*[-–—]\s*)?(?:O\/U|Over\/Under|Spread|Total|Winner|Moneyline|ML)\b.*$");
regex_exata!(SM_2, r"(?i)\s+(?:1st|2nd|First|Second)\s+Half\b.*$");
regex_exata!(SM_3, r"(?i)\s+(?:Over|Under)\s+[-+]?\d+(?:[.,]\d+)?\b.*$");

/// `stripMarketSuffix(v)`.
fn strip_market_suffix(v: &str) -> String {
    let x = clean_team(v);
    let x = SM_1.replace(&x, "");
    let x = SM_2.replace(&x, "");
    let x = SM_3.replace(&x, "");
    clean_team(&x)
}

regex_exata!(MF_PERGUNTA, r"(?i)\s*[–—-]\s*(?:Will|Does|Can)\b.*$");
regex_exata!(MF_VS, r"(?i)(.+?)\s+(?:vs\.?|versus|@)\s+(.+?)$");
regex_exata!(MF_B1, r"(?i)\s*[-–—]\s+.+?\b(?:O\/U|Over\/Under|Spread|Total|Winner|To Advance|Moneyline|ML)\b.*$");
regex_exata!(
    MF_B2,
    r"(?i)\s*(?:[–—-])\s*(?:O\/U|Over\/Under|Spread|Total|Winner|To Advance|Moneyline|ML|More\s+Markets|(?:1st|2nd|First|Second)\s+Half|(?:Over|Under)\s+\d+(?:[.,]\d+)?)\b.*$"
);
regex_exata!(MF_B3, r"(?i)\s*(?:[–—-])\s*(?:Will|Does|Can)\b.*$");
regex_exata!(MF_B4, r"(?i)\s+(?:Winner\??|To Advance|To Win|Advances?)(?:\s*\?)?$");
regex_exata!(MF_B5, r"(?i)\s+More\s+Markets\s*$");
regex_exata!(MF_B6, r"(?i)\s*[–—-]\s*(?:Yes|No)\s*$");
regex_exata!(MF_B7, r"(?i)\s*[-–—:]\s*Halftime Result\??$");

/// `matchupFrom(v)`.
fn matchup_from(v: &str) -> String {
    let fonte = MF_PERGUNTA.replace(v, "");
    let Some(m) = MF_VS.captures(js::trim_js(&fonte)) else { return String::new() };
    let a = strip_market_suffix(&m[1]);
    let mut b = strip_market_suffix(&m[2]);
    if let Some(i) = b.find(':') {
        b = trim(&b[..i]);
    }
    for re in [&MF_B1, &MF_B2, &MF_B3, &MF_B4, &MF_B5, &MF_B6, &MF_B7] {
        b = re.replace(&b, "").into_owned();
    }
    if !a.is_empty() && !b.is_empty() { format!("{a} vs {b}") } else { String::new() }
}

regex_exata!(MT, r"(?i)^(.+?)\s+vs\s+(.+)$");

/// `matchupTeams(match)`.
fn matchup_teams(confronto: &str) -> Vec<String> {
    MT.captures(confronto).map(|c| vec![trim(&c[1]), trim(&c[2])]).unwrap_or_default()
}

regex_exata!(NU_ESCANTEIOS, r"\b(corner|corners|corner kicks|escanteios)\b");
regex_exata!(NU_GOLS, r"\b(goal|goals|gols)\b");
regex_exata!(NU_RUNS, r"\b(run|runs)\b|baseball|mlb");
regex_exata!(NU_PONTOS, r"\b(point|points)\b");

/// `normalizeUnit(text)`.
fn normalize_unit(texto: &str, slug: &str) -> &'static str {
    let oficial = titles::unit(texto, slug);
    if !oficial.is_empty() {
        return oficial;
    }
    let x = texto.to_lowercase();
    if NU_ESCANTEIOS.is_match(&x) {
        "escanteios"
    } else if NU_GOLS.is_match(&x) {
        "gols"
    } else if NU_RUNS.is_match(&x) {
        "runs"
    } else if NU_PONTOS.is_match(&x) {
        "pontos"
    } else {
        ""
    }
}

regex_exata!(EL_1, r"(?i)\b(?:over|under)\s+([0-9]+(?:[.,][0-9]+)?)");
regex_exata!(EL_2, r"(?i)\b(?:O\/U|Over\/Under|Total)\s*([0-9]+(?:[.,][0-9]+)?)");

/// `extractLine(texts)`.
fn extract_line(textos: &[String]) -> String {
    for t in textos {
        if let Some(m) = EL_1.captures(t).or_else(|| EL_2.captures(t)) {
            return m[1].replacen(',', ".", 1);
        }
    }
    String::new()
}

regex_exata!(OU, r"(?i)\b(over|under)\b");

/// `extractOU()`: o resultado da posição manda, nunca o título do grupo.
fn extract_ou(resultado: &str, lado: &str) -> String {
    OU.captures(resultado).or_else(|| OU.captures(lado)).map(|c| c[1].to_lowercase()).unwrap_or_default()
}

regex_exata!(EP_T5, r"(?:first|1st)[_\s-]*(?:5|five)[_\s-]*innings?");
regex_exata!(EP_5, r"(?i)\b(?:first|1st)\s*(?:5|five)\s+innings?\b");
regex_exata!(EP_1ST5, r"(?i)\b1st\s*5\s+innings?\b");
regex_exata!(EP_T1, r"(?:first|1st)[_\s-]*half");
regex_exata!(EP_T2, r"(?:second|2nd)[_\s-]*half");
regex_exata!(EP_1, r"(?i)\b(?:first|1st)\s+half\b");
regex_exata!(EP_1H, r"(?i)\b1H\b");
regex_exata!(EP_2, r"(?i)\b(?:second|2nd)\s+half\b");
regex_exata!(EP_2H, r"(?i)\b2H\b");

/// `extractPeriod(texts, typeHint)`.
fn extract_period(textos: &[String], tipo: &str) -> String {
    let todos: Vec<&str> = std::iter::once(tipo).chain(textos.iter().map(String::as_str)).collect();
    let oficial = titles::period(&todos.join(" "));
    if !oficial.is_empty() {
        return oficial;
    }
    let juntos = textos.join(" ");
    let tipo = tipo.to_lowercase();
    if EP_T5.is_match(&tipo) || EP_5.is_match(&juntos) || EP_1ST5.is_match(&juntos) {
        "1st 5 Innings"
    } else if EP_T1.is_match(&tipo) {
        "1st Half"
    } else if EP_T2.is_match(&tipo) {
        "2nd Half"
    } else if EP_1.is_match(&juntos) || EP_1H.is_match(&juntos) {
        "1st Half"
    } else if EP_2.is_match(&juntos) || EP_2H.is_match(&juntos) {
        "2nd Half"
    } else {
        ""
    }
    .to_string()
}

regex_exata!(GRUPO_GENERICO, r"(?i)^(?:yes|no|over|under|draw|o\/u|over\/under|total)(?:\s+[-+]?\d+(?:[.,]\d+)?)?$");

/// `isGenericGroup(g)`.
fn is_generic_group(g: &str) -> bool {
    g.is_empty() || GRUPO_GENERICO.is_match(g)
}

thread_local! {
    static TIME_OU: RefCell<HashMap<String, Regex>> = RefCell::new(HashMap::new());
}

/// `new RegExp("^\\s*" + escapeRegexLiteral(t) + "\\s*(?:[-–—:]\\s*)?(?:O\\/U|Over\\/Under)\\b", "i").test(texto)`.
fn time_com_ou(time: &str, texto: &str) -> bool {
    TIME_OU.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() > 4096 {
            cache.clear();
        }
        cache
            .entry(time.to_string())
            .or_insert_with(|| {
                let inicio = js::re_exata(r"^\s*");
                let fim = js::re_exata(r"\s*(?:[-–—:]\s*)?(?:O\/U|Over\/Under)\b");
                Regex::new(&format!("(?i){}{}{}", inicio.as_str(), regex::escape(time), fim.as_str())).expect("regex do time")
            })
            .is_match(texto)
    })
}

regex_exata!(PERGUNTA_TIME, r"(?i)^\s*(?:will|does|can)\s+(.+?)\s+(?:score|have|get|record|make|take)\b");
regex_exata!(VERBO_TIME, r"(?i)\b(?:score|have|get|record|make|take)\b");

/// O primeiro time de `times` igual a `candidato`, que o contém ou está
/// contido nele, quando não é vazio (`teams.find(...)` seguido de `if (hit)`).
fn time_parecido(times: &[String], candidato: &str) -> Option<String> {
    let c = candidato.to_lowercase();
    times
        .iter()
        .find(|t| {
            let t = t.to_lowercase();
            c == t || c.contains(&t) || t.contains(&c)
        })
        .filter(|t| !t.is_empty())
        .cloned()
}

/// `findSpecificTeam(texts, match, groupTitle)`.
fn find_specific_team(textos: &[String], confronto: &str, grupo: &str) -> String {
    let times = matchup_teams(confronto);
    if times.len() != 2 {
        return String::new();
    }
    if !is_generic_group(grupo) {
        let g = clean_team(grupo).to_lowercase();
        let achado = times.iter().find(|t| {
            let t = t.to_lowercase();
            t == g || g.starts_with(&format!("{t} "))
        });
        if let Some(t) = achado.filter(|t| !t.is_empty()) {
            return t.clone();
        }
    }
    for texto in textos.iter().filter(|t| !t.is_empty()) {
        let minusculo = texto.to_lowercase();
        if let Some(m) = PERGUNTA_TIME.captures(texto) {
            if let Some(t) = time_parecido(&times, js::trim_js(&m[1])) {
                return t;
            }
        }
        let rotulo = depois_dos_dois_pontos(texto);
        if let Some(t) = times.iter().find(|t| time_com_ou(t, &rotulo)) {
            return t.clone();
        }
        let citados: Vec<&String> = times.iter().filter(|t| !t.is_empty() && minusculo.contains(&t.to_lowercase())).collect();
        if citados.len() == 1 && VERBO_TIME.is_match(texto) {
            return citados[0].clone();
        }
    }
    String::new()
}

regex_exata!(GRUPO_GENERICO_BEISEBOL, r"(?i)^(?:yes|no|over|under|draw|o\/u|over\/under|total|spread|winner|moneyline|ml)(?:\s+[-+]?\d+(?:[.,]\d+)?)?$");

/// `norm` de `findBaseballTeam` (troca U+001F por espaço).
fn norm_beisebol(v: &str) -> String {
    js::trim_js(&v.replace('\u{1F}', " ")).to_lowercase()
}

/// `findBaseballTeam(texts, match, groupTitle)`.
fn find_baseball_team(textos: &[String], confronto: &str, grupo: &str) -> String {
    let times = matchup_teams(confronto);
    if times.len() != 2 {
        return String::new();
    }
    let g = clean_team(grupo);
    if !g.is_empty() && !GRUPO_GENERICO_BEISEBOL.is_match(&g) {
        let ng = norm_beisebol(&g);
        let achado = times.iter().find(|t| {
            let nt = norm_beisebol(t);
            nt == ng || ng.starts_with(&format!("{nt} "))
        });
        if let Some(t) = achado.filter(|t| !t.is_empty()) {
            return t.clone();
        }
    }
    for texto in textos.iter().map(|t| clean(t)).filter(|t| !t.is_empty()) {
        if let Some(m) = PERGUNTA_TIME.captures(&texto) {
            let c = norm_beisebol(&clean_team(&m[1]));
            let achado = times.iter().find(|t| {
                let nt = norm_beisebol(t);
                c == nt || c.contains(&nt) || nt.contains(&c)
            });
            if let Some(t) = achado.filter(|t| !t.is_empty()) {
                return t.clone();
            }
        }
        if let Some(t) = times.iter().find(|t| time_com_ou(t, &texto)) {
            return t.clone();
        }
    }
    String::new()
}

regex_exata!(PM_LADO, r"(?i)^\s*(yes|no)\s*[:\-]?\s*");
regex_exata!(PM_TIPO_SEP, r"[\s-]+");
regex_exata!(PM_EXTRA, r"(?i)\bwill\s+the\s+game\s+go\s+to\s+extra\s+innings\b");
regex_exata!(PM_TENIS_LEGADO, r"(?i)^(?:atp|wta|itf)(?:-doubles)?-[a-z0-9]+-[a-z0-9]+-\d{4}-\d{2}-\d{2}$");
regex_exata!(VS_INTEIRO, r"(?i)^(.+?)\s+vs\.?\s+(.+)$");
regex_exata!(PM_AMERICANO_LEGADO, r"(?i)^(?:cfb|nfl|nba|wnba|nhl|bkbsl)-[a-z0-9]+-[a-z0-9]+-\d{4}-\d{2}-\d{2}$");
regex_exata!(PM_VS_NU, r"(?i)^(.+?)\s+vs\.?\s+([^:]+)$");
regex_exata!(PM_ESCANTEIOS, r"(?i)\b(?:O\/U|Over\/Under|Total)\s*([0-9]+(?:[.,][0-9]+)?)\s*(?:Total\s*)?(?:Corners?|Corner\s+Kicks?)\b");
regex_exata!(PM_TOTAL, r"(?i)\b(?:O\/U|Over\/Under|Total)\s*([0-9]+(?:[.,][0-9]+)?)\b");
regex_exata!(PM_DICA_PONTOS, r"(?:^|[-_\s])(?:cfb|nfl|ncaaf|cfl|ufl|xfl)(?:[-_\s]|$)|\b(?:american football|points?)\b");
regex_exata!(PM_DICA_RUNS, r"\b(?:mlb|npb|kbo|baseball|runs?)\b");
regex_exata!(PM_BTTS, r"(?i)\bboth\s+teams\s+to\s+score\b");
regex_exata!(PM_SIM_NAO, r"(?i)^(Yes|No)$");
regex_exata!(PM_PRIMEIRO_TEMPO, r"(?i)\b(?:first|1st)\s+half\b");
regex_exata!(PM_TENIS_PERGUNTA, r"(?i)^(.+?)\s+vs\.?\s+(.+?)\s*:\s*(?:Total\s+Sets|Match\s+O\/U|Set\s+1\s+Games\s+O\/U)\b");
regex_exata!(
    PM_HANDICAP_SETS,
    r"(?i)^Set\s+Handicap:\s*(.+?)\s*\(\s*([+-]\d+(?:[.,]\d+)?)\s*\)\s+vs\.?\s+(.+?)\s*\(\s*([+-]\d+(?:[.,]\d+)?)\s*\)\s*$"
);
regex_exata!(PM_REBOTES, r"(?i)^(.+?):\s*Rebounds\s+O\/U\s+(\d+(?:[.,]\d+)?)$");
regex_exata!(PM_HANDICAP_MAPA, r"(?i)^(Map|Game)\s+Handicap:\s*(.+?)\s*\(([+-]\d+(?:[.,]\d+)?)\)\s+vs\.?\s+(.+?)\s*\(([+-]\d+(?:[.,]\d+)?)\)$");
regex_exata!(PM_LINHA_SINAL, r"(?:^|\s)(?:[+-]\d+(?:[.,]\d+)?)\b");
regex_exata!(PM_SPREAD, r"(?i)\b(?:spread|handicap)\b");
regex_exata!(PM_SPREAD_TIPO, r"(?i)^(?:spread|handicap)$");
regex_exata!(PM_EXTRA_INNINGS, r"(?i)\bextra\s+innings\b");
regex_exata!(PM_RESOLVE, r"(?i)\bresolve\s+to\b");
regex_exata!(PM_SE, r"(?i)\bif\b");
regex_exata!(PM_ALVO_INVALIDO, r"(?i)^(?:yes|no|over|under|draw)$");
regex_exata!(PM_ALVO_ML_INVALIDO, r"(?i)^(?:yes|no|over|under)$");
regex_exata!(PM_TENIS_SLUG, r"(?i)^(?:atp|wta|itf)(?:-doubles)?-");
regex_exata!(PM_TENIS_TEXTO, r"(?i)\btennis\b");
regex_exata!(PM_AMERICANO_SLUG, r"(?i)^(?:cfb|ncaaf|nfl|cfl|ufl|xfl)-");
regex_exata!(PM_LIDERANDO, r"(?i)^(?:Will\s+)?(.+?)\s+(?:be\s+)?leading\s+at\s+halftime\??$");
regex_exata!(PM_SIM_NAO_MIN, r"(?i)^(?:yes|no)$");
regex_exata!(PM_BEISEBOL_SLUG, r"(?i)^(?:mlb|npb|kbo|cpbl|milb)-");
regex_exata!(PM_INNING, r"(?i)inning");
regex_exata!(PM_EMPATE_INNING, r"(?i)\btie(?:s)?\b|First Inning Run\??$");
regex_exata!(PM_AMERICANO_VENCE, r"(?i)^(?:Will\s+)?(.+?)\s+win(?:s)?(?:\s+the)?\s+((?:1st|2nd|first|second)\s+half|[1-4](?:st|nd|rd|th)\s+quarter)\??$");
regex_exata!(PM_BEISEBOL_TEXTO, r"(?i)\b(?:baseball|mlb|npb|kbo|cpbl|milb)\b");
regex_exata!(PM_INNINGS_RUNS, r"(?i)\b(?:innings?|runs?)\b");
regex_exata!(SP_1, r"(?i)(?:1st\s*5|first\s*5)\s+innings?\s+spread\s*[:\-–—]?\s*(.+?)\s*\(\s*([+-]?\d+(?:[.,]\d+)?)\s*\)");
regex_exata!(SP_2, r"(?i)(?:1st\s+half|first\s+half)\s+spread\s*[:\-–—]?\s*(.+?)\s*\(\s*([+-]?\d+(?:[.,]\d+)?)\s*\)");
regex_exata!(SP_3, r"(?i)(?:spread|handicap)\s*[:\-–—]?\s*(.+?)\s*\(\s*([+-]?\d+(?:[.,]\d+)?)\s*\)");
regex_exata!(SP_4, r"(?i)(?:1st\s*5|first\s*5)\s+innings?\s+spread\s*[:\-–—]?\s*(.+?)\s+([+-]?\d+(?:[.,]\d+)?)(?:\s|$)");
regex_exata!(SP_5, r"(?i)(?:1st\s+half|first\s+half)\s+spread\s*[:\-–—]?\s*(.+?)\s+([+-]?\d+(?:[.,]\d+)?)(?:\s|$)");
regex_exata!(SP_6, r"(?i)(?:spread|handicap)\s*[:\-–—]?\s*(.+?)\s+([+-]?\d+(?:[.,]\d+)?)(?:\s|$)");
regex_exata!(SP_GRUPO, r"^(.+?)\s+([+-]\d+(?:[.,]\d+)?)$");
regex_exata!(SP_LINHA_META, r"[+-]?\d+(?:[.,]\d+)?");
regex_exata!(SP_LINHA, r"([+-]\d+(?:[.,]\d+)?)");
regex_exata!(AV_TITULO, r"(?i)\bto advance\b|\badvances?\b");
regex_exata!(AV_LADO, r"(?i)\badvance");
regex_exata!(AV_SUFIXO, r"(?i)\s+(?:advances?|to advance)$");
regex_exata!(AV_PREFIXO, r".*?:\s*");
regex_exata!(AV_TO_ADVANCE, r"(?i)\s+to advance\??$");
regex_exata!(VENCE_PERGUNTA, r"(?i)\bwill\s+.+?\s+win\b");
regex_exata!(VENCE_CAPTURA, r"(?i)\bwill\s+(.+?)\s+win\b");
regex_exata!(MAIS_MERCADOS, r"(?i)\bMore\s+Markets\b\s*[-–—:]\s*(.+)$");
regex_exata!(INTERROGACOES_FINAIS, r"\?+$");
regex_exata!(ESPACOS_FINAIS, r"\s+$");
regex_exata!(VENCEDOR_TITULO, r"(?i)\b(?:winner|to win|moneyline)\b|^ML\b");
regex_exata!(VENCEDOR_LADO, r"(?i)\b(?:to win|wins?)\b");
regex_exata!(VENCEDOR_SUFIXO, r"(?i)\s+(?:to win|wins?|is the winner)$");
regex_exata!(CC_TRECHO, r"(?i)^[\s\S]*?\s+(?:vs\.?|versus|@)\s+[\s\S]*?:\s*");
regex_exata!(CC_1, r"^\s*[:\-–—]\s*");
regex_exata!(CC_2, r"(?i)^More\s+Markets\s*(?:[:\-–—]\s*)?");
regex_exata!(CC_3, r"^\s*[-–—]\s*");
regex_exata!(CC_4, r"(?i)\bMore\s+Markets\b\s*[-–—:]?\s*");
regex_exata!(CC_5, r"\s{2,}");
regex_exata!(CC_WINNER, r"(?i)^winner$");

/// `norm` do handicap de mapas: NFKC, sem caracteres invisíveis, minúsculo e
/// só letras e números (`/[^\p{L}\p{N}]/gu`).
fn norm_handicap(v: &str) -> String {
    static NAO_ALFANUMERICO: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| Regex::new(r"[^\p{L}\p{N}]").expect("regex"));
    let sem_invisiveis: String = v.nfkc().filter(|c| !matches!(c, '\u{200B}'..='\u{200F}' | '\u{2060}' | '\u{FEFF}')).collect();
    NAO_ALFANUMERICO.replace_all(&sem_invisiveis.to_lowercase(), "").into_owned()
}

/// Um mercado de `markets` escolhido quando é o único que passa no teste.
fn unico(mercados: &[Value], teste: impl Fn(&Value) -> bool) -> Option<&Value> {
    let achados: Vec<&Value> = mercados.iter().filter(|m| teste(m)).collect();
    if achados.len() == 1 { Some(achados[0]) } else { None }
}

/// `normalizarNomeMercadoPolymarket(d)`. `Err` quando o JS lançaria exceção.
pub fn normalizar_polymarket(d: &Value) -> R<Value> {
    if !js::truthy(Some(d)) || titles::plataforma(d, "polymarket") != "polymarket" {
        return Ok(titulo_ou_vazio(d));
    }
    let empate = exc(titles::draw_title(d))?;
    if !empate.is_empty() {
        return Ok(texto(empate));
    }
    let raw = clean_v(d.get("title"));
    if raw.is_empty() {
        return Ok(texto(""));
    }
    let luta = exc(titles::combat_title(d))?;
    if !luta.is_empty() {
        return Ok(texto(luta));
    }
    normalizar_polymarket_corpo(d, raw).map(texto)
}

fn normalizar_polymarket_corpo(d: &Value, raw: String) -> R<String> {
    let outcome_raw = clean_v(d.get("outcome"));
    let side = trim(&PM_LADO.replace(&outcome_raw, ""));
    let meta_esporte = d.get("_sportMeta");
    let lista = |k: &str| -> &[Value] { meta_esporte.and_then(|m| m.get(k)).and_then(Value::as_array).map_or(&[], Vec::as_slice) };
    let meta_texts = lista("texts");
    let market_meta = lista("markets");
    let condition_id = aparado(ou(d.get("conditionId"), d.get("condition_id"))).to_lowercase();
    let market_slug = aparado(d.get("slug")).to_lowercase();
    let asset_id = aparado(ou(ou(d.get("asset"), d.get("assetId")), d.get("tokenId")));

    let mut selecionado: Option<&Value> = None;
    if !asset_id.is_empty() {
        let alvo = sem_aspas(&asset_id);
        selecionado = unico(market_meta, |m| {
            m.get("tokenIds").and_then(Value::as_array).is_some_and(|ids| ids.iter().any(|t| sem_aspas(js::trim_js(&js::texto(Some(t)))) == alvo))
        });
    }
    if selecionado.is_none() && !condition_id.is_empty() {
        selecionado = unico(market_meta, |m| aparado(ou(m.get("conditionId"), m.get("condition_id"))).to_lowercase() == condition_id);
    }
    if selecionado.is_none() && !market_slug.is_empty() {
        selecionado = unico(market_meta, |m| aparado(m.get("slug")).to_lowercase() == market_slug);
    }
    let campo = |k: &str| selecionado.and_then(|m| m.get(k));
    let selected_texts: Vec<String> = match selecionado {
        Some(m) => ["question", "groupItemTitle", "sportsMarketType", "description", "category"]
            .iter()
            .filter_map(|k| m.get(*k).filter(|v| js::truthy(Some(v))))
            .map(|v| clean(&js::texto(Some(v))))
            .collect(),
        None => Vec::new(),
    };
    let market_question = clean_v(campo("question"));
    let market_type = PM_TIPO_SEP.replace_all(&js::texto_ou_vazio(campo("sportsMarketType")).to_lowercase(), "_").into_owned();
    let group_item_title = clean_v(campo("groupItemTitle"));

    let mut confronto = String::new();
    if PM_EXTRA.is_match(&market_question) {
        let partes: Vec<&str> = DOIS_PONTOS.split(&market_question).collect();
        let depois = trim(&partes[1..].join(": "));
        if !depois.is_empty() {
            confronto = matchup_from(&depois);
        }
    }
    if confronto.is_empty() {
        confronto = matchup_from(&raw);
        for t in meta_texts {
            if !confronto.is_empty() {
                break;
            }
            confronto = matchup_from(&js::texto_ou_vazio(Some(t)));
        }
        for t in &selected_texts {
            if !confronto.is_empty() {
                break;
            }
            confronto = matchup_from(t);
        }
    }
    let estendido = exc(titles::extended_title(d, &confronto))?;
    if !estendido.is_empty() {
        return Ok(estendido);
    }
    if confronto.is_empty() {
        return Ok(raw);
    }

    // Slugs-base de ATP/WTA/ITF sem a resposta Gamma: só o vencedor da partida.
    if !js::truthy(meta_esporte) && PM_TENIS_LEGADO.is_match(&market_slug) {
        let fonte = match raw.rfind(':') {
            Some(i) => js::trim_js(&raw[i + 1..]),
            None => raw.as_str(),
        };
        if let Some(m) = VS_INTEIRO.captures(fonte) {
            let (casa, fora, alvo) = (clean_team(&m[1]), clean_team(&m[2]), clean_team(&outcome_raw));
            let norm = |v: &str| js::trim_js(v).to_lowercase();
            if !alvo.is_empty() && (norm(&alvo) == norm(&casa) || norm(&alvo) == norm(&fora)) {
                return Ok(format!("{casa} vs {fora} - {alvo} ML"));
            }
        }
    }

    if selecionado.is_none() && PM_AMERICANO_LEGADO.is_match(&market_slug) {
        if let Some(b) = PM_VS_NU.captures(&raw) {
            let alvo = outcome_raw.to_lowercase();
            if [&b[1], &b[2]].iter().any(|t| js::trim_js(t).to_lowercase() == alvo) {
                return Ok(format!("{confronto} - {outcome_raw} ML"));
            }
        }
    }

    // Só sem a Gamma: totais e escanteios legados pelo título bruto.
    if !js::truthy(meta_esporte) {
        let prefixo_e_time = |inicio: usize| {
            let prefixo = depois_dos_dois_pontos(&raw[..inicio]);
            let minusculo = prefixo.to_lowercase();
            let time = matchup_teams(&confronto).into_iter().find(|t| minusculo.starts_with(&t.to_lowercase())).unwrap_or_default();
            (prefixo, time)
        };
        let com_espaco = |s: &str| if s.is_empty() { String::new() } else { format!("{s} ") };
        if let (Some(e), Some(o)) = (PM_ESCANTEIOS.captures(&raw), OU.captures(&outcome_raw)) {
            let linha = e[1].replacen(',', ".", 1);
            let (prefixo, time) = prefixo_e_time(e.get(0).map_or(0, |m| m.start()));
            let periodo = extract_period(&[prefixo], "");
            return Ok(format!("{confronto} - {}{}{} {linha} escanteios", com_espaco(&time), com_espaco(&periodo), cap(&o[1])));
        }
        if let (Some(t), Some(o)) = (PM_TOTAL.captures(&raw), OU.captures(&outcome_raw)) {
            let (prefixo, time) = prefixo_e_time(t.get(0).map_or(0, |m| m.start()));
            let dica = [d.get("eventSlug"), d.get("slug"), d.get("sport"), d.get("category")]
                .into_iter()
                .filter(|v| js::truthy(*v))
                .map(js::texto)
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase();
            let da_categoria = unidade_da_categoria(d)?;
            let unidade = match titles::unit(&raw, &market_slug) {
                "" if !da_categoria.is_empty() => da_categoria,
                "" if PM_DICA_PONTOS.is_match(&dica) => "pontos",
                "" if PM_DICA_RUNS.is_match(&dica) => "runs",
                "" => "gols",
                u => u,
            };
            let periodo = extract_period(&[prefixo], "");
            return Ok(format!("{confronto} - {}{}{} {} {unidade}", com_espaco(&time), com_espaco(&periodo), cap(&o[1]), t[1].replacen(',', ".", 1)));
        }
    }

    let texto_exato = std::iter::once(market_question.clone()).chain(selected_texts.iter().cloned()).collect::<Vec<_>>().join(" ");
    // Ambas marcam: Yes/No e período só do mercado exato.
    if selecionado.is_some() && PM_BTTS.is_match(&texto_exato) {
        if let Some(sn) = PM_SIM_NAO.captures(&outcome_raw) {
            let tempo = if PM_PRIMEIRO_TEMPO.is_match(&texto_exato) { " 1st Half" } else { "" };
            return Ok(format!("{confronto} - Both Teams to Score{tempo} {}", cap(&sn[1])));
        }
    }

    // Tênis: só o mercado exato define período, linha e lado.
    if let Some(sel) = selecionado {
        let confronto_tenis = PM_TENIS_PERGUNTA
            .captures(&market_question)
            .map(|c| format!("{} vs {}", clean_team(&c[1]), clean_team(&c[2])))
            .unwrap_or_else(|| confronto.clone());
        let ou_tenis = extract_ou(&outcome_raw, &side);
        let mut linha_tenis = extract_line(std::slice::from_ref(&market_question));
        if linha_tenis.is_empty() {
            if let Some(l) = sel.get("line").filter(|v| !v.is_null()) {
                linha_tenis = js::texto(Some(l)).replacen(',', ".", 1);
            }
        }
        let tem = !ou_tenis.is_empty() && !linha_tenis.is_empty();
        match market_type.as_str() {
            "tennis_set_totals" if tem => return Ok(format!("{confronto_tenis} - {} {linha_tenis} sets", cap(&ou_tenis))),
            "tennis_match_totals" if tem => return Ok(format!("{confronto_tenis} - {} {linha_tenis} games", cap(&ou_tenis))),
            "tennis_first_set_totals" if tem => return Ok(format!("{confronto_tenis} - 1st Set: {} {linha_tenis} games", cap(&ou_tenis))),
            "tennis_set_handicap" => {
                if let Some(h) = PM_HANDICAP_SETS.captures(&market_question) {
                    let (casa, fora, alvo) = (clean_team(&h[1]), clean_team(&h[3]), clean_team(&outcome_raw));
                    let norm = |v: &str| js::trim_js(v).to_lowercase();
                    let linha = if norm(&alvo) == norm(&casa) {
                        &h[2]
                    } else if norm(&alvo) == norm(&fora) {
                        &h[4]
                    } else {
                        ""
                    };
                    if !linha.is_empty() {
                        return Ok(format!("{casa} vs {fora} - Handicap de sets: {alvo} {}", linha.replacen(',', ".", 1)));
                    }
                }
            }
            _ => {}
        }
    }

    if selecionado.is_some() && market_type == "rebounds" {
        if let Some(p) = PM_REBOTES.captures(&market_question) {
            let ou = extract_ou(&outcome_raw, &side);
            let selecao = if ou.is_empty() { format!("Rebotes O/U {} {outcome_raw}", &p[2]) } else { format!("{} {} rebotes", cap(&ou), &p[2]) };
            return Ok(format!("{confronto} - {} {selecao}", js::trim_js(&p[1])));
        }
    }
    if selecionado.is_some() && market_type == "map_handicap" {
        if let Some(h) = PM_HANDICAP_MAPA.captures(&market_question) {
            // O apelido precisa ser único; senão fica a pergunta oficial e a seleção.
            let alvo = norm_handicap(&outcome_raw);
            let acertos = [norm_handicap(&h[2]), norm_handicap(&h[4])].map(|t| t == alvo || (js::tamanho_utf16(&t) >= 3 && alvo.starts_with(&t)));
            if acertos.iter().filter(|a| **a).count() == 1 {
                let linha = if acertos[0] { &h[3] } else { &h[5] };
                let unidade = if h[1].to_lowercase() == "map" { "mapas" } else { "games" };
                return Ok(format!("{} vs {} - Handicap de {unidade}: {outcome_raw} {linha}", clean_team(&h[2]), clean_team(&h[4])));
            }
            return Ok(format!("{market_question} - Seleção: {outcome_raw}"));
        }
    }

    // Spread/handicap só pelo mercado exato, o título bruto e o lado da posição.
    let spread_sources: Vec<String> = selected_texts.iter().cloned().chain(std::iter::once(raw.clone())).filter(|s| !s.is_empty()).collect();
    let spread_text = spread_sources.join(" | ");
    let dica_linha = PM_LINHA_SINAL.is_match(&raw) || PM_LINHA_SINAL.is_match(&outcome_raw) || PM_LINHA_SINAL.is_match(&side);
    let e_spread = PM_SPREAD.is_match(&spread_text) || PM_SPREAD_TIPO.is_match(&market_type) || dica_linha;

    if PM_EXTRA_INNINGS.is_match(&texto_exato) && PM_EXTRA.is_match(&texto_exato) {
        if let Some(sn) = PM_SIM_NAO.captures(&outcome_raw) {
            return Ok(format!("{confronto} - Extra Innings {}", cap(&sn[1])));
        }
    }
    if !e_spread && market_type != "moneyline" && PM_RESOLVE.is_match(&texto_exato) && PM_SE.is_match(&texto_exato) {
        let alvo = clean_team(&outcome_raw);
        if !alvo.is_empty() && !PM_ALVO_INVALIDO.is_match(&alvo) {
            return Ok(format!("{confronto} - {alvo} ML"));
        }
    }

    if let Some(sel) = selecionado.filter(|_| market_type == "moneyline") {
        // O resultado original: `side` tira o "No" de times como "Notre Dame".
        let alvo = clean_team(&outcome_raw);
        if !alvo.is_empty() && !PM_ALVO_ML_INVALIDO.is_match(&alvo) {
            let dica_tenis = [d.get("sport"), d.get("category"), sel.get("category")]
                .into_iter()
                .filter(|v| js::truthy(*v))
                .map(js::texto)
                .collect::<Vec<_>>()
                .join(" ");
            let tenis = PM_TENIS_SLUG.is_match(&market_slug) || PM_TENIS_TEXTO.is_match(&dica_tenis);
            let fonte = match market_question.rfind(':') {
                Some(i) => js::trim_js(&market_question[i + 1..]),
                None => market_question.as_str(),
            };
            let exibido = tenis
                .then(|| VS_INTEIRO.captures(fonte))
                .flatten()
                .map(|c| format!("{} vs {}", clean_team(&c[1]), clean_team(&c[2])))
                .unwrap_or_else(|| confronto.clone());
            let americano = PM_AMERICANO_SLUG.is_match(&market_slug) || categoria_e(d, "Futebol Americano")?;
            return Ok(if americano {
                format!("{exibido} - {}", titles::football_winner(&alvo, &selected_texts.join(" "), None))
            } else {
                format!("{exibido} - {alvo} ML")
            });
        }
    }

    // Com o mercado exato, o título bruto nunca define período, time, linha ou unidade.
    for q in std::iter::once(&market_question).chain(selected_texts.iter()).filter(|q| !q.is_empty()) {
        let q = clean(q);
        if let Some(m) = PM_LIDERANDO.captures(&q) {
            let lado = js::trim_js(&outcome_raw);
            if PM_SIM_NAO_MIN.is_match(lado) {
                let time = clean_team(&m[1]);
                if !time.is_empty() {
                    return Ok(format!("{confronto} - 1st Half {time} ML{}", if lado.to_lowercase() == "no" { " No" } else { "" }));
                }
            }
        }
    }
    let exato: Vec<String> = if selecionado.is_some() { selected_texts.clone() } else { Vec::new() };
    let analise = exato.join(" ");
    let beisebol_binario = selecionado.is_some() && PM_BEISEBOL_SLUG.is_match(&market_slug) && PM_SIM_NAO_MIN.is_match(&outcome_raw);
    if beisebol_binario && PM_INNING.is_match(&market_question) && PM_EMPATE_INNING.is_match(&market_question) {
        let detalhe = titles::kalshi_detail_js(&json!({ "title": market_question }), &outcome_raw.to_lowercase(), &Escopo::de_texto(&market_type), false);
        if !detalhe.is_empty() {
            return Ok(format!("{confronto} - {detalhe}"));
        }
    }
    let americano = selecionado.is_some() && (PM_AMERICANO_SLUG.is_match(&market_slug) || categoria_e(d, "Futebol Americano")?);
    if americano {
        if let Some(q) = PM_AMERICANO_VENCE.captures(&market_question) {
            if PM_SIM_NAO_MIN.is_match(&outcome_raw) {
                return Ok(format!("{confronto} - {}", titles::football_winner(&clean_team(&q[1]), &q[2], Some(&outcome_raw.to_lowercase()))));
            }
        }
    }
    let ou = extract_ou(&outcome_raw, &side);
    let linha = extract_line(&exato);
    // Unidade explícita do mercado ganha; o esporte só para um total sem unidade.
    let do_esporte = if selecionado.is_some() { unidade_da_categoria(d)? } else { "" };
    let unidade = [
        normalize_unit(&analise, &market_slug),
        if selecionado.is_some() { normalize_unit(&js::texto_ou_vazio(campo("category")), &market_slug) } else { "" },
        do_esporte,
    ]
    .into_iter()
    .find(|u| !u.is_empty())
    .unwrap_or("gols");
    let periodo = extract_period(&exato, &market_type);
    let beisebol = PM_BEISEBOL_TEXTO.is_match(&analise) || PM_INNINGS_RUNS.is_match(&analise);
    let time = match beisebol {
        true => Some(find_baseball_team(&exato, &confronto, &group_item_title)).filter(|t| !t.is_empty()),
        false => None,
    }
    .unwrap_or_else(|| find_specific_team(&exato, &confronto, &group_item_title));
    if !ou.is_empty() && !linha.is_empty() {
        let partes = [time.as_str(), periodo.as_str(), &cap(&ou), &linha, unidade];
        return Ok(format!("{confronto} - {}", partes.iter().filter(|p| !p.is_empty()).copied().collect::<Vec<_>>().join(" ")));
    }

    if e_spread {
        let times = matchup_teams(&confronto);
        let periodo_spread = extract_period(&spread_sources, &market_type);
        // O time do rótulo é sempre o do resultado da posição.
        let time_resultado = strip_market_suffix(&side);
        let time_spread = time_parecido(&times, &time_resultado).unwrap_or_else(|| time_resultado.clone());
        // O time a que o texto da linha se refere, para saber se inverte o sinal.
        let mut time_do_texto = String::new();
        let mut linha_spread = String::new();
        for fonte in &spread_sources {
            if let Some(m) = [&SP_1, &SP_2, &SP_3, &SP_4, &SP_5, &SP_6].iter().find_map(|re| re.captures(fonte)) {
                if let Some(t) = time_parecido(&times, &strip_market_suffix(&m[1])) {
                    time_do_texto = t;
                }
                linha_spread = m[2].replacen(',', ".", 1);
            }
            if !time_do_texto.is_empty() && !linha_spread.is_empty() {
                break;
            }
        }
        if let Some(g) = SP_GRUPO.captures(&js::texto_ou_vazio(campo("groupItemTitle"))) {
            if let Some(t) = time_parecido(&times, &strip_market_suffix(&g[1])) {
                time_do_texto = t;
            }
            linha_spread = g[2].replacen(',', ".", 1);
        }
        if linha_spread.is_empty() {
            if let Some(l) = campo("line").filter(|v| !v.is_null()) {
                if let Some(m) = SP_LINHA_META.find(&js::texto(Some(l))) {
                    linha_spread = m.as_str().replacen(',', ".", 1);
                }
            }
        }
        if linha_spread.is_empty() {
            if let Some(c) = SP_LINHA.captures(&spread_text) {
                linha_spread = c[1].replacen(',', ".", 1);
            }
        }
        if !time_spread.is_empty() && !linha_spread.is_empty() {
            let mut n = js::number(Some(&Value::String(linha_spread.clone())));
            if !time_do_texto.is_empty() && time_do_texto.to_lowercase() != time_spread.to_lowercase() && n.is_finite() {
                n = -n;
            }
            let texto_linha = match js::numero_texto(n) {
                Some(t) => format!("{}{t}", if n > 0.0 { "+" } else { "" }),
                None => {
                    let sinal = linha_spread.starts_with('+') || linha_spread.starts_with('-');
                    format!("{}{linha_spread}", if !sinal && js::number(Some(&Value::String(linha_spread.clone()))) > 0.0 { "+" } else { "" })
                }
            };
            let alvo = if periodo_spread.is_empty() { time_spread } else { format!("{time_spread} {periodo_spread}") };
            return Ok(format!("{confronto} - {alvo} {texto_linha}"));
        }
    }

    if AV_TITULO.is_match(&raw) || AV_LADO.is_match(&side) {
        let mut time = trim(&AV_SUFIXO.replace(&side, ""));
        if time.is_empty() {
            time = trim(&AV_TO_ADVANCE.replace(&AV_PREFIXO.replace(&raw, ""), ""));
        }
        return Ok(format!("{confronto} - {} Advances", if time.is_empty() { &side } else { &time }));
    }
    // Vencedor só pelo mercado exato e nunca numa posição de spread.
    if !e_spread {
        if let Some(pergunta) = std::iter::once(&raw).chain(exato.iter()).find(|t| VENCE_PERGUNTA.is_match(t)) {
            if let Some(m) = VENCE_CAPTURA.captures(pergunta) {
                let time = strip_market_suffix(&m[1]);
                if !time.is_empty() {
                    return Ok(format!("{confronto} - {time} ML"));
                }
            }
        }
    }
    if let Some(m) = MAIS_MERCADOS.captures(&raw) {
        let s = format!("{confronto} - {}", js::trim_js(&INTERROGACOES_FINAIS.replace(&m[1], " ")));
        return Ok(ESPACOS_FINAIS.replace(&s, " ").into_owned());
    }
    if !e_spread && (VENCEDOR_TITULO.is_match(&raw) || VENCEDOR_LADO.is_match(&side)) {
        let time = trim(&VENCEDOR_SUFIXO.replace(&side, ""));
        if !time.is_empty() {
            return Ok(format!("{confronto} - {time} ML"));
        }
    }
    // O confronto exibido troca "vs." por "vs": nunca é índice no título bruto.
    let trecho = CC_TRECHO.find(&raw);
    let indice = indice_utf16(&raw.to_lowercase(), &confronto.to_lowercase());
    let resto = match (trecho, indice) {
        (Some(t), _) => &raw[t.end()..],
        (None, Some(i)) => js::resto_utf16(&raw, i + js::tamanho_utf16(&confronto)),
        (None, None) => return Ok(raw),
    };
    let c = trim(&CC_1.replace(resto, ""));
    let c = CC_2.replace(&c, "");
    let c = CC_3.replace(&c, "");
    let c = trim(&INTERROGACOES_FINAIS.replace(&c, " "));
    let c = CC_4.replace_all(&c, "");
    let c = trim(&CC_5.replace_all(&c, " "));
    Ok(if !c.is_empty() && !CC_WINNER.is_match(&c) { format!("{confronto} - {c}") } else { confronto })
}

regex_exata!(KN_AMERICANO, r"(?i)^KX(?:NCAAF|NFL|CFL|UFL|XFL)");
regex_exata!(KN_VS_PONTO, r"\bvs\.");
regex_exata!(KN_VS, r"(?i)\s+vs\s+");
regex_exata!(KN_VENCE, r"(?i)^(.+?)\s+wins?\??$");
regex_exata!(KN_OU_SOLTO, r"(?i)^Will\s+(over|under)\s+([0-9]+(?:[.,]\d+)?)\s+(goals?|runs?|points?)\s+be\s+scored\??$");
regex_exata!(KN_LADO_EXPLICITO, r"(?i):\s*(Yes|No)$");
regex_exata!(KN_TEMPO, r"(?i)^(.+?)\s+wins?\s+(1st|first|2nd|second)\s+half$");
regex_exata!(KN_PRIMEIRO, r"(?i)^(?:1st|first)$");
regex_exata!(KN_AVANCA, r"(?i)^(.+?)\s+advances?$");
regex_exata!(KN_TEMPO_NORMAL, r"(?i)^Reg(?:ulation)?\s*Time\s*[:\-–—]\s*(.+)$");
regex_exata!(KN_CINCO_ENTRADAS, r"(?i)^(over|under)\s+([0-9]+(?:[.,]\d+)?)\s+(runs?|goals?|points?)\s+in\s+the\s+first\s+5\s+innings?$");
regex_exata!(KN_DO_TIME, r"(?i)^(.+?)\s+(over|under)\s+([0-9]+(?:[.,]\d+)?)\s+(runs?|goals?|points?)\s+scored$");
regex_exata!(KN_SO_LINHA, r"(?i)^(over|under)\s+([0-9]+(?:[.,]\d+)?)$");

/// `unitFromWord(w)`.
fn unit_from_word(w: &str) -> &'static str {
    let x = w.to_lowercase();
    if x.starts_with("run") {
        "runs"
    } else if x.starts_with("goal") {
        "gols"
    } else if x.starts_with("point") {
        "pontos"
    } else {
        ""
    }
}

/// `normalizarNomeMercadoKalshi(d)`. `Err` quando o JS lançaria exceção.
pub fn normalizar_kalshi(d: &Value) -> R<Value> {
    if !js::truthy(Some(d)) || titles::plataforma(d, "") != "kalshi" {
        return Ok(titulo_ou_vazio(d));
    }
    let especial = titles::kalshi_special_title_linha(d);
    if !especial.is_empty() {
        return Ok(texto(especial));
    }
    let entrada = exc(titles::inning_winner_title_linha(d))?;
    if !entrada.is_empty() {
        return Ok(texto(entrada));
    }
    if let Some(meta) = d.get("_kalshiTitleMeta").filter(|m| js::truthy(Some(m))) {
        let id = js::texto_ou_vazio(ou(ou(d.get("ticker"), d.get("market_ticker")), d.get("slug")));
        let americano = KN_AMERICANO.is_match(&id) || categoria_e(d, "Futebol Americano")?;
        let lado = js::texto_ou_vazio(d.get("outcome")).to_lowercase();
        let detalhe = titles::kalshi_detail_js(meta.get("market").unwrap_or(&Value::Null), &lado, &Escopo::de_valor(meta.get("scope")), americano);
        let oficial = KN_VS_PONTO.replace_all(&js::texto_ou_vazio(meta.get("matchup")), "vs").into_owned();
        let titulo = js::texto_ou_vazio(d.get("title"));
        let bruto = KN_VS_PONTO.replace_all(titulo.split(" - ").next().unwrap_or(""), "vs").into_owned();
        let oficiais: Vec<&str> = KN_VS.split(&oficial).collect();
        let existentes: Vec<&str> = KN_VS.split(&bruto).collect();
        // Mantém a grafia e a ordem; só completa um prefixo cortado quando o
        // participante oficial prova sozinho os caracteres que faltam.
        let restaurados: Vec<String> = existentes
            .iter()
            .map(|time| {
                let (n, fim) = (js::tamanho_utf16(time), time.to_lowercase());
                let candidatos: Vec<&&str> = oficiais
                    .iter()
                    .filter(|t| {
                        let m = js::tamanho_utf16(t);
                        m > n && m - n <= 3 && t.to_lowercase().ends_with(&fim)
                    })
                    .collect();
                if candidatos.len() == 1 { candidatos[0].to_string() } else { time.to_string() }
            })
            .collect();
        let confronto = if existentes.len() == 2 { restaurados.join(" vs ") } else { oficial.clone() };
        if !detalhe.is_empty() && !confronto.is_empty() {
            return Ok(texto(format!("{confronto} - {detalhe}")));
        }
        let titulo_mercado = js::texto_ou_vazio(meta.get("market").and_then(|m| m.get("title")));
        if let Some(w) = KN_VENCE.captures(&titulo_mercado).filter(|_| !confronto.is_empty()) {
            let nao = js::texto(d.get("outcome")).to_lowercase() == "no";
            return Ok(texto(format!("{confronto} - {} ML{}", &w[1], if nao { " No" } else { "" })));
        }
    }
    let raw = clean_v(d.get("title"));
    if raw.is_empty() {
        return Ok(texto(""));
    }
    // Eventos sem o nome dos times em nenhum campo: ao menos o formato Over/Under.
    if let Some(b) = KN_OU_SOLTO.captures(&raw) {
        return Ok(texto(format!("{} {} {}", cap(&b[1]), b[2].replacen(',', ".", 1), match unit_from_word(&b[3]) {
            "" => "gols",
            u => u,
        })));
    }
    normalizar_kalshi_corpo(d, raw).map(texto)
}

fn normalizar_kalshi_corpo(d: &Value, raw: String) -> R<String> {
    let (parte_confronto, complemento) = match raw.find(" - ") {
        Some(i) => (js::trim_js(&raw[..i]), js::trim_js(&raw[i + 3..])),
        None => (raw.as_str(), ""),
    };
    if complemento.is_empty() {
        return Ok(raw.clone());
    }
    let Some(vs) = VS_INTEIRO.captures(parte_confronto) else { return Ok(raw.clone()) };
    let times = [trim(&vs[1]), trim(&vs[2])];
    let confronto = format!("{} vs {}", times[0], times[1]);
    let outcome_raw = Some(clean_v(d.get("outcome"))).filter(|o| !o.is_empty()).unwrap_or_else(|| "Yes".into());
    let pergunta_compacta = match KN_LADO_EXPLICITO.captures(complemento) {
        Some(c) if c[1].to_lowercase() == outcome_raw.to_lowercase() => js::trim_js(&complemento[..c.get(0).map_or(0, |m| m.start())]),
        _ => complemento,
    };
    let zero = json!(0);
    let versao = js::number(ou(d.get("_kalshiTitleFormatVersion"), Some(&zero)));
    if versao < 4.0 {
        let total = titles::compact_kalshi_total(pergunta_compacta, &outcome_raw, "");
        if !total.is_empty() {
            return Ok(format!("{confronto} - {total}"));
        }
    }
    let find_team = |candidato: &str| -> String {
        let c = js::trim_js(candidato);
        if c.is_empty() { String::new() } else { time_parecido(&times, c).unwrap_or_default() }
    };
    let ou_texto = |m: &str| cap(m);
    if let Some(m) = KN_TEMPO.captures(complemento) {
        let time = Some(find_team(&m[1])).filter(|t| !t.is_empty()).unwrap_or_else(|| trim(&m[1]));
        let periodo = if KN_PRIMEIRO.is_match(&m[2]) { "1st Half" } else { "2nd Half" };
        return Ok(format!("{confronto} - {time} {periodo} {outcome_raw}"));
    }
    if let Some(m) = KN_AVANCA.captures(complemento) {
        let time = Some(find_team(&m[1])).filter(|t| !t.is_empty()).unwrap_or_else(|| trim(&m[1]));
        return Ok(format!("{confronto} - {time} Advances"));
    }
    if let Some(m) = KN_TEMPO_NORMAL.captures(complemento) {
        let time = Some(find_team(&m[1])).filter(|t| !t.is_empty()).unwrap_or_else(|| trim(&m[1]));
        return Ok(format!("{confronto} - {time} ML{}", if outcome_raw.to_lowercase() == "no" { " No" } else { "" }));
    }
    if let Some(m) = KN_CINCO_ENTRADAS.captures(complemento) {
        let unidade = Some(unit_from_word(&m[3])).filter(|u| !u.is_empty()).unwrap_or("runs");
        return Ok(format!("{confronto} - 1st 5 Innings {} {} {unidade}", ou_texto(&m[1]), m[2].replacen(',', ".", 1)));
    }
    if let Some(m) = KN_DO_TIME.captures(complemento) {
        let time = Some(find_team(&m[1])).filter(|t| !t.is_empty()).unwrap_or_else(|| trim(&m[1]));
        let unidade = Some(unit_from_word(&m[4])).filter(|u| !u.is_empty()).unwrap_or("runs");
        return Ok(format!("{confronto} - {time} {} {} {unidade}", ou_texto(&m[2]), m[3].replacen(',', ".", 1)));
    }
    // Sem unidade explícita não dá para saber a modalidade: fica o título recebido.
    if KN_SO_LINHA.is_match(complemento) {
        return Ok(raw.clone());
    }
    // Só o nome de um dos times: vencedor da partida (ML).
    let so_time = find_team(complemento);
    let (c, t) = (complemento.to_lowercase(), so_time.to_lowercase());
    if !so_time.is_empty() && (t == c || c.ends_with(&format!(" {t}"))) {
        return Ok(format!("{confronto} - {complemento} ML{}", if outcome_raw.to_lowercase() == "no" { " No" } else { "" }));
    }
    Ok(raw.clone())
}

/// `tituloExibicao(d)`. `Err` quando o JS lançaria exceção.
pub fn titulo_exibicao(d: &Value) -> R<Value> {
    if !js::truthy(Some(d)) {
        return Ok(texto(""));
    }
    if d.get("_platform").and_then(Value::as_str) == Some("predict") {
        if let Some(t) = d.get("_predictTitle").filter(|t| js::truthy(Some(t))) {
            return Ok(t.clone());
        }
    }
    let empate = exc(titles::draw_title(d))?;
    if !empate.is_empty() {
        return Ok(texto(empate));
    }
    let estendido = exc(titles::extended_title(d, ""))?;
    if !estendido.is_empty() {
        return Ok(texto(estendido));
    }
    let especial = titles::kalshi_special_title_linha(d);
    if !especial.is_empty() {
        return Ok(texto(especial));
    }
    let entrada = exc(titles::inning_winner_title_linha(d))?;
    if !entrada.is_empty() {
        return Ok(texto(entrada));
    }
    if let Some(t) = d.get("_titleDisplay").filter(|t| js::truthy(Some(t))) {
        if !titles::kalshi_title_missing_matchup(d, Some(&clean_v(Some(t)))) {
            return Ok(t.clone());
        }
    }
    // Títulos são só apresentação: uma exceção na normalização vira o título bruto.
    let normalizado = match titles::plataforma(d, "polymarket").as_str() {
        "polymarket" => normalizar_polymarket(d),
        "kalshi" => normalizar_kalshi(d),
        _ => Ok(texto("")),
    };
    Ok(normalizado.ok().filter(|v| js::truthy(Some(v))).unwrap_or_else(|| titulo_ou_vazio(d)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titulo(d: Value) -> String {
        titulo_exibicao(&d).unwrap().as_str().unwrap().to_string()
    }

    #[test]
    fn exemplos() {
        // Sem ":" e com "vs." o confronto não é achado no título bruto: fica o título.
        assert_eq!(titulo(json!({ "_platform": "polymarket", "title": "Lakers vs. Celtics", "outcome": "Lakers" })), "Lakers vs. Celtics");
        assert_eq!(titulo(json!({ "_platform": "polymarket", "title": "Lakers vs. Celtics: O/U 220.5", "outcome": "Over" })), "Lakers vs Celtics - Over 220.5 gols");
        assert_eq!(
            titulo(json!({ "_platform": "polymarket", "title": "Arsenal vs. Chelsea: Spread", "outcome": "Arsenal", "slug": "x",
                "_sportMeta": { "markets": [{ "slug": "x", "question": "Spread: Chelsea (-1.5)", "sportsMarketType": "spreads" }] } })),
            "Arsenal vs Chelsea - Arsenal +1.5"
        );
        assert_eq!(titulo(json!({ "_platform": "kalshi", "title": "Yankees vs Red Sox - Yankees", "outcome": "no" })), "Yankees vs Red Sox - Yankees ML No");
        assert_eq!(titulo(json!({ "_platform": "predict", "_predictTitle": "Predito" })), "Predito");
        assert_eq!(titulo(json!({ "_platform": "sxbet", "title": "SX" })), "SX");
        assert_eq!(titulo(json!(null)), "");
    }

    #[test]
    fn excecoes_como_no_js() {
        // Mercado nulo: `exactMarket()` lança fora do `try`.
        assert_eq!(titulo_exibicao(&json!({ "slug": "x", "_sportMeta": { "markets": [null] } })), Err(Excecao));
        // Dentro do `try` (aqui no espalhamento de `texts`), fica o título bruto.
        let d = json!({ "title": "A vs B", "outcome": "Over", "slug": "x",
            "_sportMeta": { "texts": 5, "markets": [{ "slug": "x", "question": "Over 2.5 rounds" }] } });
        assert_eq!(titulo_exibicao(&d), Ok(json!("A vs B")));
    }

    #[test]
    fn utf16_como_no_js() {
        assert_eq!(indice_utf16("ab😀cd", "cd"), Some(4));
        assert_eq!(js::resto_utf16("ab😀cd", 4), "cd");
        assert_eq!(js::tamanho_utf16("😀"), 2);
    }

    /// Oráculo contra o `tituloExibicao()` real: os casos e as respostas do JS
    /// vêm de `ferramentas/validacao/oraculo_titulos.mjs`
    /// (`BETTY_ORACULO_TITULOS=<arquivo .jsonl>`).
    #[test]
    #[ignore]
    fn oraculo_titulos_exibicao() {
        let caminho = std::env::var("BETTY_ORACULO_TITULOS").expect("defina BETTY_ORACULO_TITULOS");
        let texto = std::fs::read_to_string(caminho).unwrap();
        let (mut total, mut diferentes, mut exemplos) = (0usize, 0usize, Vec::new());
        for linha in texto.lines().filter(|l| !l.trim().is_empty()) {
            let caso: Value = serde_json::from_str(linha).unwrap();
            let esperado = if caso.get("erro").is_some() { Err(Excecao) } else { Ok(caso["saida"].clone()) };
            let obtido = titulo_exibicao(&caso["entrada"]);
            total += 1;
            if obtido != esperado {
                diferentes += 1;
                if exemplos.len() < 25 {
                    exemplos.push(format!("{}: JS {:?} x Rust {:?}", caso["id"], esperado, obtido));
                }
            }
        }
        assert!(diferentes == 0, "{diferentes} de {total} casos diferentes; os primeiros:\n{}", exemplos.join("\n"));
        eprintln!("{total} casos iguais ao JS");
    }
}
