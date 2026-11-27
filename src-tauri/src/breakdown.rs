//! Agrupamento de mercados para as estatísticas, igual a
//! `public/market-breakdown.js` (`describe` e `key`). Só apresentação: nunca
//! altera valores financeiros.
//!
//! As regex foram escritas na sintaxe do JS original. `js()` converte as
//! diferenças relevantes: no JavaScript `\b` e `\d` são ASCII, no crate
//! `regex` são Unicode.

use std::sync::LazyLock;

use regex::Regex;
use unicode_normalization::UnicodeNormalization;

use crate::js::re as js;

macro_rules! regex {
    ($nome:ident, $padrao:expr) => {
        crate::regex_js!($nome, $padrao);
    };
}

regex!(ESPACOS, r"\s+");
regex!(LONGO_PRAZO_VERBO, r"(?i)\b(?:win|reach|qualify|advance)\b");
regex!(COMPETICAO, r"(?i)\b(?:FIFA World Cup|World Cup|Champions League|Europa League|Conference League|Libertadores|Sudamericana|Premier League|La Liga|Serie A|Bundesliga|Ligue 1|Copa do Mundo)\b");
regex!(ANO, r"\b(?:19|20)\d{2}(?:\s*[-/]\s*(?:\d{2}|\d{4}))?\b");
regex!(PERIODO, r"(?i)\b(?:[12](?:st|nd) Half|[12]H|[1-4]Q|[1-4](?:st|nd|rd|th) Quarter|[1-4][º°] quarto|\d+(?:st|nd|rd|th) Period|[1-3][º°] per[ií]odo|1st \d+ Innings|[12](?:st|nd) Innings|\d+(?:st|nd|rd|th|º) (Inning|inning|set|Map)|1st Set|Set \d+|Extra Innings|Overtime|Extra Time)\b");
regex!(INNING_NUM, r"(?i)^(\d+)(?:st|nd|rd|th|º|°)\s+inning$");
regex!(PRIMEIRAS_INNINGS, r"(?i)^1st \d+ innings$");
regex!(INNINGS_PALAVRA, r"(?i)innings");
regex!(SET_NUM, r"(?i)^(?:(\d+)(?:st|nd|rd|th|º) set|set (\d+))$");
regex!(PERIODO_HOQUEI, r"(?i)^([1-3])(?:(?:st|nd|rd) Period|[º°] per[ií]odo)$");
regex!(NO_FINAL, r"(?i)\bNo\s*$");
regex!(YES_FINAL, r"(?i)\bYes\s*$");
regex!(PROPS_JOGADOR, r"(?i)\b(?:Chutes no alvo|Assistências|Strikeouts)\s+(?:O/U|Over|Under)\s+\d");
regex!(METODO_VITORIA, r"(?i)Vitória por (nocaute|KO/TKO|TKO|KO|finalização|decisão)\s+(Yes|No)$");
regex!(PODIO, r"(?i)\bPódio\s+(Yes|No)$");
regex!(YES_NO_FINAL, r"(?i)(Yes|No)$");
regex!(TORNEIO_ML, r"(?i)^Torneio\s+.+?\s+ML(?:\s+No)?$");
regex!(PROP_BASQUETE, r"(?i)^(.+?)\s+(?:rebotes|rebounds?|assist[eê]ncias|assists?|steals?|blocks?|three.pointers?|3.pointers?|PRA)\s+(?:O/U|Over|Under)\s+\d+(?:[.,]\d+)?(?:\s+(?:Yes|No))?$");
regex!(VS, r"(?i)\s+vs\.?\s+");
regex!(DOIS_PONTOS_TOTAL, r"(?i):\s*(?:Over|Under|O/U)\b");
regex!(ESTATISTICA_BASQUETE, r"(?i)\b(?:points?|pontos|rebounds?|rebotes|assists?|assist[eê]ncias|steals?|blocks?|three.pointers?|3.pointers?|PRA)\b");
regex!(ML_FINAL, r"(?i)\bML\s*(?:No)?$");
regex!(ML_INICIO, r"(?i)^ML\s+");
regex!(ADVANCES, r"(?i)\bAdvances\s*(?:No)?$");
regex!(AMBAS_MARCAM, r"(?i)\bBoth Teams to Score\b");
regex!(EMPATE, r"(?i)\bEmpate\b");
regex!(EXTRA_INNINGS, r"(?i)^Extra Innings\s+(?:Yes|No)$");
regex!(NO_FIM, r"(?i)No$");
regex!(TOTAL, r"(?i)\b(Over|Under|O/U)\s+(\d+(?:[.,]\d+)?)\s+(?:(?:[12]H|[1-4]Q)\s+)?(gols|goals?|pontos|points?|runs|escanteios|corners|games|sets|rebotes|rebounds|yards|rounds?|mapas|maps|wickets|strokes)\b");
regex!(DOIS_PONTOS_OU_ESPACO, r"[:\s]+");
regex!(ALVO_JOGADOR, r"(?i)rebotes|rebounds|yards");
regex!(ALVO_TIME, r"(?i)gols|goals|escanteios|corners");
regex!(HANDICAP, r"(?i)\b(?:Handicap|Spread)\b");
regex!(LINHA_FINAL, r"\s[+-]\d+(?:\.\d+)?\s*$");
regex!(LINHA, r"[+-]\d+(?:\.\d+)?");
regex!(HANDICAP_SETS, r"(?i)handicap de sets");
regex!(HANDICAP_GAMES, r"(?i)handicap de games");
regex!(HANDICAP_MAPAS, r"(?i)handicap de mapas");
regex!(RACE, r"(?i)\bRace to\s+(\d+)\s+pontos\b");
regex!(QUALQUER_EQUIPE, r"(?i)\bQualquer equipe\b");
regex!(NENHUMA_EQUIPE, r"(?i)\bNenhuma equipe\b");
regex!(FIRST_TD, r"(?i)\bfirst TD\b");
regex!(TOTAL_INCOMPLETO, r"(?i)^(Over|Under)\s+(\d+(?:[.,]\d+)?)$");
regex!(VENCE, r"(?i)^.+?\s+wins?(?:\s+the)?(?:\s+(?:[12](?:st|nd) Half|[1-4](?:st|nd|rd|th) Quarter))?\s*(?:Yes|No)?$");
regex!(VENCE_PERIODO, r"(?i)^.+?\s+(?:[12](?:st|nd) Half|[1-4](?:st|nd|rd|th) Quarter|[1-4]º quarto)\s+(?:Yes|No)$");
regex!(GOVERNADOR_BR, r"\b(?:governor|governador)\s+(?:of|de|do|da)\s+(?:parana|sao paulo|rio de janeiro|minas gerais|rio grande do sul|santa catarina|bahia|pernambuco|ceara|maranhao|goias|mato grosso|espirito santo|amazonas|para|piaui|sergipe|alagoas|acre|roraima|rondonia|amapa|tocantins|rio grande do norte|paraiba|distrito federal)\b");

const PAISES: &[(&str, &str)] = &[
    ("Brasil", r"brazil|brazilian|brasil|brasileir[oa]"),
    ("Estados Unidos", r"united states|american|u\.s\.|usa|u\.s\.a\."),
    ("Argentina", "argentina|argentine|argentinian"),
    ("Canadá", "canada|canadian"),
    ("México", "mexico|mexican"),
    ("Reino Unido", "united kingdom|british|uk|britain"),
    ("França", "france|french"),
    ("Alemanha", "germany|german"),
    ("Itália", "italy|italian"),
    ("Espanha", "spain|spanish"),
    ("Portugal", "portugal|portuguese"),
    ("Rússia", "russia|russian"),
    ("Ucrânia", "ukraine|ukrainian"),
    ("China", "china|chinese"),
    ("Japão", "japan|japanese"),
    ("Índia", "india|indian"),
    ("Austrália", "australia|australian"),
    ("Israel", "israel|israeli"),
    ("Irã", "iran|iranian"),
    ("Turquia", "turkey|turkiye|turkish"),
    ("Coreia do Sul", "south korea|south korean"),
    ("Coreia do Norte", "north korea|north korean"),
    ("Chile", "chile|chilean"),
    ("Colômbia", "colombia|colombian"),
    ("Peru", "peru|peruvian"),
    ("Venezuela", "venezuela|venezuelan"),
    ("Uruguai", "uruguay|uruguayan"),
    ("Equador", "ecuador|ecuadorian"),
    ("Bolívia", "bolivia|bolivian"),
    ("Países Baixos", "netherlands|dutch"),
    ("Polônia", "poland|polish"),
    ("Romênia", "romania|romanian"),
    ("Hungria", "hungary|hungarian"),
    ("África do Sul", "south africa|south african"),
    ("Taiwan", "taiwan|taiwanese"),
];

/// O JS usa `(?=$|[^a-z])`; como aqui só importa se casa ou não, o lookahead
/// vira um grupo comum (o crate `regex` não tem lookaround).
static PADROES_PAISES: LazyLock<Vec<(&'static str, Regex)>> = LazyLock::new(|| {
    PAISES
        .iter()
        .map(|(nome, aliases)| {
            let p = format!("(?i)(?:^|[^a-z])(?:{aliases}|{})(?:$|[^a-z])", sem_acentos_minusculo(nome));
            (*nome, js(&p))
        })
        .collect()
});

/// `normalize('NFD').replace(/[\u0300-\u036f]/g,'').toLowerCase()`.
fn sem_acentos_minusculo(texto: &str) -> String {
    texto.nfd().filter(|c| !('\u{0300}'..='\u{036f}').contains(c)).collect::<String>().to_lowercase()
}

pub fn pais_politico(titulo: &str) -> String {
    let texto = sem_acentos_minusculo(titulo);
    let mut achados: Vec<&str> = Vec::new();
    for (nome, padrao) in PADROES_PAISES.iter() {
        if padrao.is_match(&texto) && !achados.contains(nome) {
            achados.push(nome);
        }
    }
    // Eleições estaduais são identificadas pela jurisdição, nunca por candidatos.
    if GOVERNADOR_BR.is_match(&texto) && !achados.contains(&"Brasil") {
        achados.push("Brasil");
    }
    match achados.len() {
        1 => achados[0].to_string(),
        0 => "País não identificado".into(),
        _ => "Múltiplos países".into(),
    }
}

fn sufixo_ordinal(n: u64) -> &'static str {
    if (11..=13).contains(&(n % 100)) {
        return "th";
    }
    match n % 10 {
        1 => "st",
        2 => "nd",
        3 => "rd",
        _ => "th",
    }
}

/// Nome canônico do período, com a mesma precedência do mapa montado no JS.
fn periodo_canonico(periodo: &str) -> String {
    if let Some(c) = PERIODO_HOQUEI.captures(periodo) {
        let n: usize = c[1].parse().unwrap_or(0);
        return format!("{}{} Period", n, ["", "st", "nd", "rd"][n]);
    }
    if let Some(c) = SET_NUM.captures(periodo) {
        let n: u64 = c.get(1).or(c.get(2)).map_or("0", |m| m.as_str()).parse().unwrap_or(0);
        return format!("{n}{} Set", sufixo_ordinal(n));
    }
    if PRIMEIRAS_INNINGS.is_match(periodo) {
        return INNINGS_PALAVRA.replace(periodo, "Innings").into_owned();
    }
    if let Some(c) = INNING_NUM.captures(periodo) {
        let n: u64 = c[1].parse().unwrap_or(0);
        return format!("{n}{} Inning", sufixo_ordinal(n));
    }
    let minusculo = periodo.to_lowercase();
    let fixo = match minusculo.as_str() {
        "1h" | "1st half" => Some("1st Half"),
        "2h" | "2nd half" => Some("2nd Half"),
        "extra innings" => Some("Extra Innings"),
        "overtime" => Some("Overtime"),
        "extra time" => Some("Extra Time"),
        _ => None,
    };
    if let Some(f) = fixo {
        return f.into();
    }
    for (n, nome) in [(1, "1st Quarter"), (2, "2nd Quarter"), (3, "3rd Quarter"), (4, "4th Quarter")] {
        let aliases = [nome.to_lowercase(), format!("{n}q"), format!("{n}º quarto"), format!("{n}° quarto")];
        if aliases.contains(&minusculo) {
            return nome.into();
        }
    }
    periodo.into()
}

/// `text.slice(0, text.indexOf(' - '))`, ou vazio quando não há confronto.
fn confronto(texto: &str) -> &str {
    texto.find(" - ").map_or("", |i| &texto[..i])
}

fn participantes(texto: &str) -> Vec<String> {
    VS.split(confronto(texto)).map(|t| t.trim().to_lowercase()).collect()
}

const CATEGORIAS_SEM_LINHA: [&str; 8] =
    ["MMA & Boxe", "Golfe", "Automobilismo", "Esports", "Críquete", "Rugby", "Vôlei", "Outros Esportes"];

pub fn describe(titulo: &str, categoria: &str) -> String {
    let texto = ESPACOS.replace_all(titulo, " ").trim().to_string();
    if let Some(i) = texto.find("Mercado não confirmado:") {
        return format!("Outros — {}", &texto[i..]);
    }
    if categoria == "Política" {
        return pais_politico(&texto);
    }
    if categoria == "Futebol" && LONGO_PRAZO_VERBO.is_match(&texto) {
        if let Some(c) = COMPETICAO.find(&texto) {
            let nome = match c.as_str().to_lowercase().as_str() {
                "fifa world cup" | "world cup" | "copa do mundo" => "Copa do Mundo".to_string(),
                _ => c.as_str().to_string(),
            };
            let ano = ANO.find(&texto).map_or(String::new(), |a| format!(" {}", a.as_str()));
            return format!("Longo prazo — {nome}{ano}");
        }
    }

    let detalhe: &str = texto.find(" - ").map_or(texto.as_str(), |i| &texto[i + 3..]);
    let periodo = PERIODO.find(detalhe).map(|m| m.as_str());
    let escopo = periodo.map_or_else(|| "Partida inteira".to_string(), periodo_canonico);
    let negativo = if NO_FINAL.is_match(detalhe) { " — No" } else { "" };

    if ["Futebol", "Basquete", "Beisebol"].contains(&categoria) && PROPS_JOGADOR.is_match(detalhe) {
        return "Player props".into();
    }
    if categoria == "MMA & Boxe" {
        if let Some(m) = METODO_VITORIA.captures(detalhe) {
            let metodo = &m[1];
            let mut letras = metodo.chars();
            let primeira = letras.next().map(|c| c.to_uppercase().collect::<String>()).unwrap_or_default();
            return format!("Método de vitória — {primeira}{} — {}", letras.as_str(), &m[2]);
        }
    }
    if categoria == "Automobilismo" && PODIO.is_match(detalhe) {
        let lado = YES_NO_FINAL.captures(detalhe).map_or("", |c| c.get(1).map_or("", |m| m.as_str()));
        return format!("Pódio — {lado}");
    }
    if categoria == "Golfe" && TORNEIO_ML.is_match(detalhe) {
        return format!("ML — Torneio{negativo}");
    }
    if categoria == "Basquete" {
        if let Some(prop) = PROP_BASQUETE.captures(detalhe) {
            if !participantes(&texto).contains(&prop[1].trim().to_lowercase()) {
                return "Player props".into();
            }
        }
        if DOIS_PONTOS_TOTAL.is_match(detalhe) && ESTATISTICA_BASQUETE.is_match(detalhe) {
            let nomeado = detalhe.split(':').next().unwrap_or("").trim().to_lowercase();
            if !participantes(&texto).contains(&nomeado) {
                return "Player props".into();
            }
        }
    }
    if ML_FINAL.is_match(detalhe) || ML_INICIO.is_match(detalhe) {
        return format!("ML — {escopo}{negativo}");
    }
    if ADVANCES.is_match(detalhe) {
        return format!("Advances{negativo}");
    }
    if AMBAS_MARCAM.is_match(detalhe) {
        let lado = if NO_FINAL.is_match(detalhe) {
            "No"
        } else if YES_FINAL.is_match(detalhe) {
            "Yes"
        } else {
            "O/U não informado"
        };
        return format!("Ambas marcam — {escopo} — {lado}");
    }
    if EMPATE.is_match(detalhe) {
        return format!("Empate — {escopo}{negativo}");
    }
    if categoria == "Beisebol" && EXTRA_INNINGS.is_match(detalhe) {
        return format!("Extra Innings — {}", if NO_FIM.is_match(detalhe) { "No" } else { "Yes" });
    }
    if let Some(total) = TOTAL.captures(detalhe) {
        return descrever_total(&texto, detalhe, &total, periodo, &escopo, negativo, categoria);
    }
    if HANDICAP.is_match(detalhe) || LINHA_FINAL.is_match(detalhe) {
        let linha = LINHA.find(detalhe).map(|m| m.as_str());
        let tipo = if HANDICAP_SETS.is_match(detalhe) {
            "Handicap de sets"
        } else if HANDICAP_GAMES.is_match(detalhe) {
            "Handicap de games"
        } else if HANDICAP_MAPAS.is_match(detalhe) {
            "Handicap de mapas"
        } else {
            "Handicap"
        };
        let sinal = |l: &str| if l.starts_with('-') { "Negativo" } else { "Positivo" };
        if let Some(l) = linha {
            if CATEGORIAS_SEM_LINHA.contains(&categoria) || ["Hóquei", "Tênis"].contains(&categoria) {
                return format!("{tipo} {} — {escopo}{negativo}", sinal(l));
            }
            if ["Futebol", "Futebol Americano", "Basquete", "Beisebol"].contains(&categoria) && tipo == "Handicap" {
                let e = if escopo == "Partida inteira" { "Partida Inteira" } else { escopo.as_str() };
                return format!("Handicap {} — Total — {e}{negativo}", sinal(l));
            }
        }
        return format!("{tipo}{} — {escopo}{negativo}", linha.map_or(String::new(), |l| format!(" {l}")));
    }
    if RACE.is_match(detalhe) {
        let quem = if QUALQUER_EQUIPE.is_match(detalhe) {
            " qualquer equipe"
        } else if NENHUMA_EQUIPE.is_match(detalhe) {
            " nenhuma equipe"
        } else {
            ""
        };
        return format!("Race{quem} — {escopo}{negativo}");
    }
    if FIRST_TD.is_match(detalhe) {
        return format!("First TD — {escopo}{negativo}");
    }
    if let Some(c) = TOTAL_INCOMPLETO.captures(detalhe) {
        return format!("{} {} — Unidade/escopo não confirmado", &c[1], &c[2]);
    }
    if VENCE.is_match(detalhe) || (periodo.is_some() && VENCE_PERIODO.is_match(detalhe)) {
        return format!("ML — {escopo}{negativo}");
    }
    // Formatos não reconhecidos continuam visíveis em vez de virar ML por palpite.
    format!("Outros — {}", if detalhe.is_empty() { "Mercado não identificado" } else { detalhe })
}

fn descrever_total(
    texto: &str,
    detalhe: &str,
    total: &regex::Captures<'_>,
    periodo: Option<&str>,
    escopo: &str,
    negativo: &str,
    categoria: &str,
) -> String {
    let inicio = total.get(0).map_or(0, |m| m.start());
    let antes = &detalhe[..inicio];
    // `replace(period?.[0] || /$^/, '')`: remove só a primeira ocorrência do período.
    let sem_periodo = match periodo {
        Some(p) if !p.is_empty() => antes.replacen(p, "", 1),
        _ => antes.to_string(),
    };
    let prefixo = DOIS_PONTOS_OU_ESPACO.replace_all(&sem_periodo, " ").trim().to_string();
    let unidade_bruta = &total[3];
    let alvo = if prefixo.is_empty() {
        "Total"
    } else if ALVO_JOGADOR.is_match(unidade_bruta) {
        "Jogador"
    } else if ALVO_TIME.is_match(unidade_bruta) {
        "Um time"
    } else {
        "Equipe/jogador"
    };
    let minuscula = unidade_bruta.to_lowercase();
    let unidade = match minuscula.as_str() {
        "goal" | "goals" => "gols".to_string(),
        "point" | "points" => "pontos".to_string(),
        "corners" => "escanteios".to_string(),
        "rebounds" => "rebotes".to_string(),
        _ => minuscula.clone(),
    };
    let lado = match total[1].to_lowercase().as_str() {
        "over" => "Over",
        "under" => "Under",
        _ => "O/U",
    };
    let sobre_ou_sob = lado == "Over" || lado == "Under";
    let prefixo_min = prefixo.to_lowercase();

    if CATEGORIAS_SEM_LINHA.contains(&categoria) && lado != "O/U" {
        if categoria == "MMA & Boxe" && unidade == "gols" {
            return format!("{lado} {} — Unidade não confirmada", &total[2]);
        }
        return format!("{lado} {unidade} — {alvo} — {escopo}{negativo}");
    }
    if categoria == "Tênis" && sobre_ou_sob && (unidade == "games" || unidade == "sets") {
        let participante = VS.split(confronto(texto)).any(|t| t.trim().to_lowercase() == prefixo_min);
        if prefixo.is_empty() || participante {
            let quem = if prefixo.is_empty() { "Total" } else { "Um jogador/par" };
            return format!("{lado} {unidade} — {quem} — {escopo}{negativo}");
        }
    }
    if categoria == "Hóquei" && sobre_ou_sob && unidade == "gols" {
        let time = VS.split(confronto(texto)).any(|t| t.trim().to_lowercase() == prefixo_min);
        if prefixo.is_empty() || time {
            let quem = if prefixo.is_empty() { "Total" } else { "Um time" };
            return format!("{lado} gols — {quem} — {escopo}{negativo}");
        }
    }
    if categoria == "Futebol Americano" && unidade == "yards" && !prefixo.is_empty() && antes.contains(':') {
        return "Player props".into();
    }
    let unidade_time = if categoria == "Beisebol" { "runs" } else { "pontos" };
    if ["Futebol Americano", "Basquete", "Beisebol"].contains(&categoria) && sobre_ou_sob && unidade == unidade_time {
        // Só identifica um time quando o participante nomeado está no confronto.
        let times = participantes(texto);
        let total_do_time = times.len() == 2 && times.contains(&prefixo_min);
        if alvo == "Total" || total_do_time {
            let quem = if total_do_time { "Um time" } else { "Total" };
            return format!("{lado} {unidade} — {quem} — {escopo}{negativo}");
        }
    }
    if categoria == "Futebol" && (unidade == "gols" || unidade == "escanteios") && lado != "O/U" {
        return format!("{lado} {unidade} — {alvo} — {escopo}{negativo}");
    }
    format!("{lado} {} {unidade} — {alvo} — {escopo}{negativo}", total[2].replacen(',', ".", 1))
}

/// `JSON.stringify([String(category || 'Outros'), describe(title, category)])`.
pub fn chave(titulo: &str, categoria: &str) -> String {
    let c = if categoria.is_empty() { "Outros" } else { categoria };
    serde_json::to_string(&[c.to_string(), describe(titulo, categoria)]).unwrap_or_default()
}

/// `JSON.stringify([category, describe(title, category), labelFor(odd)])`.
pub fn chave_resumo(titulo: &str, categoria: &str, odd: f64) -> String {
    serde_json::to_string(&[categoria.to_string(), describe(titulo, categoria), crate::odds::rotulo(odd)])
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compara com as saídas do `market-breakdown.js` original. Rodar com
    /// `BETTY_ORACULO_BREAKDOWN=<arquivo.jsonl> cargo test -- --ignored oraculo`.
    #[test]
    #[ignore]
    fn oraculo_js() {
        let caminho = std::env::var("BETTY_ORACULO_BREAKDOWN").expect("defina BETTY_ORACULO_BREAKDOWN");
        let texto = std::fs::read_to_string(caminho).unwrap();
        let (mut total, mut erros) = (0, Vec::new());
        for linha in texto.lines() {
            let v: serde_json::Value = serde_json::from_str(linha).unwrap();
            let (t, c, o) = (v["t"].as_str().unwrap(), v["c"].as_str().unwrap(), v["o"].as_f64().unwrap_or(0.0));
            total += 1;
            if let Some(label) = v.get("label") {
                let (r, l) = (crate::odds::arredondar(o), crate::odds::rotulo(o));
                if r != v["round"].as_f64().unwrap() || l != label.as_str().unwrap() {
                    erros.push(format!("odd {o}: rust ({r}, {l}) js ({}, {label})", v["round"]));
                }
                continue;
            }
            let (k, s) = (chave(t, c), chave_resumo(t, c, o));
            if k != v["key"].as_str().unwrap() {
                erros.push(format!("key [{c}] {t:?}\n   rust {k}\n   js   {}", v["key"].as_str().unwrap()));
            } else if s != v["summary"].as_str().unwrap() {
                erros.push(format!("summary [{c}] {t:?} odd {o}\n   rust {s}\n   js   {}", v["summary"].as_str().unwrap()));
            }
        }
        for e in erros.iter().take(25) {
            eprintln!("{e}");
        }
        assert!(erros.is_empty(), "{} de {total} divergem do JS", erros.len());
        eprintln!("{total} casos idênticos ao JS");
    }

    #[test]
    fn exemplos_por_tipo_de_mercado() {
        let casos = [
            ("Flamengo vs Palmeiras - Flamengo ML", "Futebol", "ML — Partida inteira"),
            ("Flamengo vs Palmeiras - Flamengo ML No", "Futebol", "ML — Partida inteira — No"),
            ("Lakers vs Celtics - 1st Half Over 110.5 pontos", "Basquete", "Over pontos — Total — 1st Half"),
            ("Lakers vs Celtics - Lakers Over 110.5 pontos", "Basquete", "Over pontos — Um time — Partida inteira"),
            ("A vs B - Over 2.5 gols", "Futebol", "Over gols — Total — Partida inteira"),
            ("A vs B - 2º quarto Over 40.5 pontos", "Basquete", "Over pontos — Total — 2nd Quarter"),
            ("A vs B - Handicap -1.5", "Futebol", "Handicap Negativo — Total — Partida Inteira"),
            ("A vs B - Empate", "Futebol", "Empate — Partida inteira"),
            ("Will Lula win the 2026 Brazilian election?", "Política", "Brasil"),
            ("USA vs China trade deal", "Política", "Múltiplos países"),
            ("Will France win the 2026 FIFA World Cup?", "Futebol", "Longo prazo — Copa do Mundo 2026"),
            ("A vs B - Vitória por KO/TKO Yes", "MMA & Boxe", "Método de vitória — KO/TKO — Yes"),
            ("A vs B - Race to 20 pontos No", "Basquete", "Race — Partida inteira — No"),
            ("A vs B - 7th Inning Over 0.5 runs", "Beisebol", "Over runs — Total — 7th Inning"),
            ("A vs B - Algo estranho", "Futebol", "Outros — Algo estranho"),
            ("Mercado X - Mercado não confirmado: pergunta", "Futebol", "Outros — Mercado não confirmado: pergunta"),
        ];
        for (titulo, categoria, esperado) in casos {
            assert_eq!(describe(titulo, categoria), esperado, "{titulo} / {categoria}");
        }
    }

    #[test]
    fn chave_json_igual_ao_node() {
        assert_eq!(chave("A vs B - A ML", ""), r#"["Outros","Outros — A ML"]"#.replace("Outros — A ML", &describe("A vs B - A ML", "")));
        assert_eq!(chave_resumo("x", "Futebol", 1.5), format!(r#"["Futebol","{}","1.41 – 1.60"]"#, describe("x", "Futebol")));
    }
}
