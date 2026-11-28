//! Categoria de uma aposta, igual a `classificarAposta()` do `dashboard.js`,
//! com `sinaisDaAposta`, `normalizarEsporte`, `categoriaPorTema`,
//! `esportePorTicker`, `esportePorTiposMercado`, `slugDeConfronto`,
//! `metadadoDizEsporte`, `esportePorConfronto` e `temaDoMercadoProprio`.
//!
//! É a primeira peça da sincronização no Rust (item "Sincronização inteira no
//! Rust" de `docs/desempenho.md`): ainda não é usada pelas rotas, e o oráculo
//! (`tests::oraculo_categorias`) compara com o JS real.
//!
//! Diferenças do Rust que importam aqui, e como ficam iguais ao JS:
//! - `\b` do JS sem a flag `u` só conhece letras ASCII (`js::re`);
//! - `\s` e `trim()` do JS incluem U+FEFF e não incluem U+0085;
//! - `String([a, b])` é `"a,b"`;
//! - onde o JS lança exceção (um mercado nulo em `_sportMeta.markets`, um
//!   `sportsMarketTypes` que não é iterável...), o resultado é `Err`.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use crate::{js, titles};

pub const OUTROS: &str = "Outros";
pub const ESPORTE_GENERICO: &str = "Outros Esportes";

/// O JS lançaria exceção.
#[derive(Debug, PartialEq, Eq)]
pub struct Excecao;

/// `\s` do JavaScript.
const ESPACO_JS: &str = r"[\t\n\x0B\x0C\r \u{A0}\u{1680}\u{2000}-\u{200A}\u{2028}\u{2029}\u{202F}\u{205F}\u{3000}\u{FEFF}]";

fn espaco_js(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'..='\u{200A}' | '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' | '\u{3000}' | '\u{FEFF}')
}

/// `String.prototype.trim()`.
fn trim_js(s: &str) -> &str {
    s.trim_matches(espaco_js)
}

fn re(padrao: &str) -> Regex {
    js::re(&padrao.replace(r"\s", ESPACO_JS))
}

macro_rules! regex_cat {
    ($nome:ident, $padrao:expr) => {
        static $nome: LazyLock<Regex> = LazyLock::new(|| re($padrao));
    };
}

/// `String(v)` do JavaScript, inclusive para listas.
fn texto_js(v: &Value) -> String {
    match v {
        Value::Array(itens) => itens.iter().map(|i| if i.is_null() { String::new() } else { texto_js(i) }).collect::<Vec<_>>().join(","),
        outro => js::string(Some(outro)),
    }
}

/// `v != null && String(v).trim() !== ""`, já como texto.
fn texto_preenchido(v: Option<&Value>) -> Option<String> {
    let v = v.filter(|v| !v.is_null())?;
    let t = texto_js(v);
    (!trim_js(&t).is_empty()).then_some(t)
}

// `normalizarEsporte()`.
regex_cat!(NE_CLUBE, r"\b(?:fc|afc|cf|sc|fk|ac|cd|ud|united|city|athletic|sporting|calcio)\b");
regex_cat!(NE_AMERICANO, r"american\s+football|futebol\s+americano|\bnfl\b|\bncaaf\b|\bcfl\b|\bcfb\b|college\s+football|\bufl\b|\busfl\b|\bxfl\b");
regex_cat!(NE_TENIS, r"tennis|t[eê]nis|\batp\b|\bwta\b|\bitf\b|grand\s+slam|australian\s+open|roland\s+garros|french\s+open|us\s+open|wimbledon|davis\s+cup|billie\s+jean\s+king");
regex_cat!(NE_BASQUETE, r"basketball|basquete|\bnba\b|\bwnba\b|\bncaab\b|euroleague|eurocup|\bacb\b|liga\s+endesa|\bnbl\b|\bcba\b|b\.league|\bfiba\b");
regex_cat!(NE_BEISEBOL, r"baseball|beisebol|\bmlb\b|\bnpb\b|\bkbo\b|\bcpbl\b|\bmilb\b|world\s+baseball\s+classic");
regex_cat!(NE_HOQUEI, r"hockey|h[oó]quei|\bnhl\b|\bahl\b|\bkhl\b|\bshl\b|\bliiga\b|ice\s+hockey");
regex_cat!(NE_MMA, r"\bufc\b|\bmma\b|bellator|\bpfl\b|mixed\s+martial|boxing|\bboxe\b|heavyweight\s+title|one\s+championship");
regex_cat!(NE_GOLFE, r"\bgolf\b|\bgolfe\b|\bpga\b|liv\s+golf|ryder\s+cup|the\s+masters\b|open\s+championship|dp\s+world\s+tour");
regex_cat!(NE_AUTO, r"formula\s*1|\bf1\b|nascar|motogp|indycar|grand\s+prix|\bwrc\b|le\s+mans");
regex_cat!(NE_ESPORTS, r"\be[-\s]?sports?\b|league\s+of\s+legends|lol\s+worlds|counter[-\s]?strike|\bcs2\b|cs:?go|\bdota\b|valorant|overwatch|rocket\s+league");
regex_cat!(NE_CRIQUETE, r"\bcricket\b|cr[ií]quete|\bipl\b|\bt20\b|\bbbl\b|test\s+match");
regex_cat!(NE_RUGBY, r"\brugby\b|six\s+nations|\bnrl\b|super\s+rugby");
regex_cat!(NE_VOLEI, r"volleyball|v[oô]lei|\bfivb\b");
regex_cat!(
    NE_FUTEBOL,
    r"soccer|futebol|\bepl\b|premier\s+league|la\s+liga|laliga|serie\s+a|serie\s+b|bundesliga|ligue\s*1|ligue\s*2|eredivisie|champions\s+league|europa\s+league|conference\s+league|libertadores|sul[-\s]?americana|copa\s+do\s+brasil|\bmls\b|\bnwsl\b|brasileirao|brasileiro|saudi\s+(?:pro|club)|\bspl\b|super\s+lig|liga\s+mx|liga\s+portugal|primeira\s+liga|championship|fa\s+cup|coppa\s+italia|dfb\s+pokal|world\s+cup|copa\s+am[ée]rica|\beuro\s+20\d\d\b|\bfifa\b|\buefa\b|conmebol"
);

/// `normalizarEsporte(v)`.
fn normalizar_esporte(v: &str) -> Option<&'static str> {
    let s = trim_js(v).to_lowercase();
    if s.is_empty() {
        return None;
    }
    let clube = NE_CLUBE.is_match(&s);
    let regras: [(&LazyLock<Regex>, &'static str, bool); 13] = [
        (&NE_AMERICANO, "Futebol Americano", false),
        (&NE_TENIS, "Tênis", true),
        (&NE_BASQUETE, "Basquete", false),
        (&NE_BEISEBOL, "Beisebol", false),
        (&NE_HOQUEI, "Hóquei", false),
        (&NE_MMA, "MMA & Boxe", false),
        (&NE_GOLFE, "Golfe", false),
        (&NE_AUTO, "Automobilismo", true),
        (&NE_ESPORTS, "Esports", false),
        (&NE_CRIQUETE, "Críquete", false),
        (&NE_RUGBY, "Rugby", false),
        (&NE_VOLEI, "Vôlei", false),
        (&NE_FUTEBOL, "Futebol", false),
    ];
    regras.into_iter().find(|(r, _, so_sem_clube)| !(*so_sem_clube && clube) && r.is_match(&s)).map(|(_, nome, _)| nome)
}

/// `normalizarEsporte(v)` com o `String(v || "")` da entrada.
fn normalizar_esporte_v(v: &Value) -> Option<&'static str> {
    if js::truthy(Some(v)) { normalizar_esporte(&texto_js(v)) } else { None }
}

static REGRAS_TEMA: LazyLock<Vec<(&'static str, Regex)>> = LazyLock::new(|| {
    [
        ("Cripto", r"\b(bitcoin|btc|ethereum|ether|eth|solana|dogecoin|doge|xrp|ripple|cardano|litecoin|chainlink|crypto|cripto|criptomoeda|altcoin|stablecoin|usdc|usdt|memecoin|nfts?|binance|coinbase|blockchain|satoshi|halving|defi|hyperliquid|onchain)\b"),
        ("Geopolítica", r"\b(war|guerra|ceasefire|cessar[-\s]?fogo|armistice|invasion|invas[ãa]o|nuclear\s+(?:weapon|test|strike)|missile|m[ií]ssil|airstrike|air\s+strike|troops|tropas|nato|otan|hostages?|ref[ée]ns?|peace\s+(?:deal|talks|plan|agreement)|acordo\s+de\s+paz|annex|sanctions?|san[çc][õo]es|coup\b|golpe\s+de\s+estado|drone\s+strike|blockade)\b"),
        ("Política", r"\b(election|elei[çc][ãa]o|elei[çc][õo]es|eleitoral|president|presidente|presidential|primary\s+(?:election|winner)|caucus|senate|senado|senador|house\s+of\s+representatives|congress|congresso|governor|governador|prime\s+minister|primeiro[-\s]?ministro|chancellor|impeachment|impeached|cabinet|nominee|nomination|ballot|electoral\s+college|swing\s+state|democrats?|republicans?|\bgop\b|trump|biden|harris|vance|newsom|obama|lula|bolsonaro|milei|maduro|macron|starmer|orb[áa]n|netanyahu|zelensky|putin|xi\s+jinping|kim\s+jong|supreme\s+court|\bstf\b|\btse\b|mayor\b|prefeito|referendum|plebiscito|parliament|parlamento|shutdown|executive\s+order|pardon|indicted?|deportations?|epstein|nobel\s+peace)\b"),
        ("Economia", r"\b(fed\b|federal\s+reserve|fomc|powell|interest\s+rates?|rate\s+(?:cut|hike)|juros|selic|\bcpi\b|inflation|infla[çc][ãa]o|\bgdp\b|\bpib\b|recession|recess[ãa]o|unemployment|desemprego|jobs\s+report|payrolls|treasury|yields?|s&p\s*500|nasdaq|dow\s+jones|ibovespa|stock\s+price|share\s+price|market\s+cap|earnings|\bipo\b|acquisition|merger|bankrupt|d[óo]lar|exchange\s+rate|oil\s+price|gold\s+price|debt\s+ceiling|tariffs?)\b"),
        ("Tecnologia & IA", r"\b(openai|anthropic|claude|chatgpt|gpt[-\s]?\d|\bgpt\b|gemini|llama\s*\d|grok|deepseek|mistral|midjourney|sora|\bagi\b|artificial\s+intelligence|intelig[êe]ncia\s+artificial|\bllms?\b|\bai\s+(?:model|lab|agent|safety|race|company)\b|arc[-\s]agi|nvidia|apple\b|iphone|android|tesla|robotaxi|self[-\s]driving|waymo|neuralink|quantum\s+comput|tiktok|twitter|meta\s+ai|google\b|microsoft|amazon\b|semiconductor)\b"),
        ("Esports", r"\b(league\s+of\s+legends|counter[-\s]?strike|\bcs2\b|cs:?go|\bdota\b|valorant|overwatch|e[-\s]?sports?)\b"),
        ("Games", r"\b(gta\b|grand\s+theft\s+auto|rockstar\s+games|steam\b|nintendo|switch\s*2|playstation|\bps5\b|xbox|elden\s+ring|zelda|pok[ée]mon|minecraft|fortnite|roblox|call\s+of\s+duty|game\s+awards|videogames?|video\s+games?|speedrun|half[-\s]life|silksong|hollow\s+knight)\b"),
        ("Entretenimento", r"\b(oscars?\b|academy\s+award|grammys?\b|emmys?\b|golden\s+globe|box\s+office|bilheteria|movie|filmes?\b|netflix|\bhbo\b|disney|spotify|billboard|albums?\b|[áa]lbum|taylor\s+swift|kanye|beyonc[ée]|rotten\s+tomatoes|\bimdb\b|halftime\s+show|miss\s+universe|big\s+brother|\bbbb\b|reality\s+show|celebrity|celebridade|div[óo]rcio|divorce|engaged|noivado|met\s+gala|person\s+of\s+the\s+year|youtube|mrbeast|podcast)\b"),
        ("Ciência & Saúde", r"\b(nasa\b|spacex|starship|falcon\s+9|rocket\s+launch|lan[çc]amento\s+espacial|asteroid|\bmars\b|moon\s+landing|eclipse|nobel\s+prize|vaccines?|vacina|covid|h5n1|bird\s+flu|gripe\s+avi[áa]ria|outbreak|surto|\bfda\b|clinical\s+trial|cancer|c[âa]ncer|cure\s+for|measles|pandemic|pandemia|\bcern\b|nuclear\s+fusion|fus[ãa]o\s+nuclear|earthquake|terremoto|volcano|vulc[ãa]o|alien|extraterrestre)\b"),
        ("Clima", r"\b(temperature|temperatura|hurricanes?|furac[ãa]o|tornado|snowfall|nevasca|rainfall|precipita[çc][ãa]o|heat\s+wave|onda\s+de\s+calor|el\s+ni[nñ]o|la\s+ni[nñ]a|weather|clima\b|wildfire|inc[êe]ndio\s+florestal|drought|seca\b|\bco2\b|global\s+warming|aquecimento\s+global|hottest\s+(?:year|month|day))\b"),
    ]
    .into_iter()
    .map(|(nome, padrao)| (nome, re(padrao)))
    .collect()
});

/// `categoriaPorTema(texto)`.
fn categoria_por_tema(texto: &str) -> Option<&'static str> {
    REGRAS_TEMA.iter().find(|(_, r)| r.is_match(texto)).map(|(nome, _)| *nome)
}

struct Sinais {
    meta: Vec<String>,
    meta_texto: String,
    slugs: Vec<String>,
    texto: String,
}

regex_cat!(TRACOS, r"[_\-]+");
regex_cat!(ESPACOS, r"\s+");

/// `limpar` de `sinaisDaAposta`.
fn limpar(s: &str) -> String {
    let s = s.to_lowercase();
    let s = TRACOS.replace_all(&s, " ");
    let s = ESPACOS.replace_all(&s, " ");
    trim_js(&s).to_string()
}

/// Os itens de `(x || [])` para `flatMap` / `filter` (só listas).
fn lista_ou_vazia(x: Option<&Value>) -> Result<&[Value], Excecao> {
    match x {
        Some(Value::Array(a)) => Ok(a),
        Some(v) if js::truthy(Some(v)) => Err(Excecao),
        _ => Ok(&[]),
    }
}

/// `sinaisDaAposta(d)`.
fn sinais_da_aposta(d: &Value) -> Result<Sinais, Excecao> {
    let meta_evento = d.get("_sportMeta");
    let mut textos_mercado = std::collections::HashSet::new();
    for m in lista_ou_vazia(meta_evento.and_then(|m| m.get("markets")))? {
        if m.is_null() {
            return Err(Excecao);
        }
        for k in ["slug", "question", "groupItemTitle", "sportsMarketType"] {
            if let Some(v) = m.get(k).filter(|v| js::truthy(Some(v))) {
                textos_mercado.insert(texto_js(v).to_lowercase());
            }
        }
    }
    let textos_meta: Vec<&Value> = match (meta_evento.and_then(|m| m.get("eventTexts")), meta_evento.and_then(|m| m.get("texts"))) {
        (Some(Value::Array(t)), _) => t.iter().collect(),
        (_, Some(Value::Array(t))) => t.iter().filter(|v| !textos_mercado.contains(&texto_js(v).to_lowercase())).collect(),
        _ => Vec::new(),
    };
    let kalshi = js::truthy(d.get("_platform")) && texto_js(&d["_platform"]).to_lowercase() == "kalshi";
    let modalidade = kalshi.then(|| titles::sport_from_kalshi_rules(d.get("_kalshiTitleMeta").and_then(|m| m.get("market")))).flatten().map(|s| Value::String(s.into()));
    let mut meta: Vec<String> = ["sport", "sportName", "category", "sportsCategory", "sportsMarketType"].iter().filter_map(|k| texto_preenchido(d.get(*k))).collect();
    meta.extend(texto_preenchido(modalidade.as_ref()));
    meta.extend(textos_meta.into_iter().filter_map(|v| texto_preenchido(Some(v))));
    let slugs: Vec<String> = ["slug", "eventSlug", "_eventTicker", "seriesTicker"].iter().filter_map(|k| texto_preenchido(d.get(*k))).collect();
    let titulo = if js::truthy(d.get("title")) { texto_js(&d["title"]) } else { String::new() };
    Ok(Sinais {
        meta_texto: limpar(&meta.join(" ")),
        texto: limpar(&format!("{titulo} {} {}", slugs.join(" "), meta.join(" "))),
        meta,
        slugs,
    })
}

regex_cat!(TK_AMERICANO, r"^KX(?:NFL|NCAAF|CFL|UFL|USFL|XFL)");
regex_cat!(TK_BEISEBOL, r"^KX(?:MLB|NPB|KBO|CPBL|MILB|WBC)");
regex_cat!(TK_BASQUETE, r"^KX(?:NBA|WNBA|NCAAB|EUROLEAGUE|ACB|NBL|CBA|BSL)");
regex_cat!(TK_BASQUETE_INTL, r"^KX(?:FIBA|DBB|BBINTL|SOWBBALL|VBA|LNBP|SESBL|JBLEAGUE|LBP)");
regex_cat!(TK_FUTEBOL_COPAS, r"^KX(?:COPADELREY|TACAPORT|IDNSL)");
regex_cat!(TK_HOQUEI, r"^KX(?:NHL|AHL|KHL|SHL|LIIGA|DEL)");
regex_cat!(TK_TENIS, r"^KX(?:ATP|WTA|ITF|TENNIS)");
regex_cat!(TK_MMA, r"^KX(?:UFC|MMA|BOX)");
regex_cat!(TK_GOLFE, r"^KX(?:PGA|LIV|GOLF|RYDER)");
regex_cat!(TK_AUTO, r"^KX(?:F1|NASCAR|MOTOGP|INDY)");
regex_cat!(TK_ESPORTS, r"^KX(?:LOL|CSGO|CS2|DOTA|VALORANT|ESPORT)");
regex_cat!(TK_CRIQUETE, r"^KX(?:CRICKET|IPL)");
regex_cat!(TK_RUGBY, r"^KX(?:RUGBY|NRL|SIXNATIONS)");
regex_cat!(
    TK_FUTEBOL,
    r"^(?:KX)?(?:AFCON|CLUBF|EPL|BUNDES|LIGUE|EREDIV|SERIEA|SERIEB|SERIEC|PERLIGA|ASEAN|UEL|UECL|UCLW?|LIBERTADOR|SULAMERICAN|COPADOBRASIL|BRASILEIRO|SAUDI|SPL|LALIGA|MLS|NWSL|CHNSL|WORLDCUP|U20WWC|ARGNACB|DIMAYOR|BOLPDIV|LEAGUESCUP|GRECUP|SRBSL|DFBPOKAL|COPPAITALIA|ETTAN|SCOCUP|SVKCUP|USL|J[123]?LEAGUE|EFLCUP|LIGAEXP|EL1|FL1)"
);
regex_cat!(TK_BK, r"^BK[A-Z0-9]+-");

/// `esportePorTicker(slugs)`.
fn esporte_por_ticker(slugs: &[String]) -> Option<&'static str> {
    let regras: [(&LazyLock<Regex>, &'static str); 15] = [
        (&TK_AMERICANO, "Futebol Americano"),
        (&TK_BEISEBOL, "Beisebol"),
        (&TK_BASQUETE, "Basquete"),
        (&TK_BASQUETE_INTL, "Basquete"),
        (&TK_FUTEBOL_COPAS, "Futebol"),
        (&TK_HOQUEI, "Hóquei"),
        (&TK_TENIS, "Tênis"),
        (&TK_MMA, "MMA & Boxe"),
        (&TK_GOLFE, "Golfe"),
        (&TK_AUTO, "Automobilismo"),
        (&TK_ESPORTS, "Esports"),
        (&TK_CRIQUETE, "Críquete"),
        (&TK_RUGBY, "Rugby"),
        (&TK_FUTEBOL, "Futebol"),
        (&TK_BK, "Basquete"),
    ];
    slugs.iter().find_map(|s| {
        let t = s.to_uppercase();
        regras.iter().find(|(r, _)| r.is_match(&t)).map(|(_, nome)| *nome)
    })
}

/// `exactMarket(d)`, com o `meta = d?._sportMeta` padrão.
fn mercado_proprio(d: &Value) -> Result<Option<&Value>, Excecao> {
    titles::exact_market(d, d.get("_sportMeta")).map_err(|()| Excecao)
}

/// `esportePorTiposMercado(d)`.
fn esporte_por_tipos_mercado(d: &Value) -> Result<Option<&'static str>, Excecao> {
    let proprio = mercado_proprio(d)?;
    let mut brutos: Vec<Option<&Value>> = vec![d.get("sportsMarketType")];
    let mut letras: Vec<Value> = Vec::new();
    match proprio {
        Some(m) => brutos.push(m.get("sportsMarketType")),
        // `...(x || [])`: lista, texto (letra por letra) ou exceção.
        None => match d.get("_sportMeta").and_then(|m| m.get("sportsMarketTypes")) {
            Some(Value::Array(a)) => brutos.extend(a.iter().map(Some)),
            Some(Value::String(s)) if !s.is_empty() => letras.extend(s.chars().map(|c| Value::String(c.to_string()))),
            Some(v) if js::truthy(Some(v)) => return Err(Excecao),
            _ => {}
        },
    }
    brutos.extend(letras.iter().map(Some));
    let tipos: Vec<String> =
        brutos.into_iter().map(|v| if js::truthy(v) { texto_js(v.unwrap_or(&Value::Null)).to_lowercase() } else { String::new() }).filter(|t| !t.is_empty()).collect();
    let tem = |prefixos: &[&str]| tipos.iter().any(|t| prefixos.iter().any(|p| t.starts_with(&format!("{p}_"))));
    Ok([
        (&["tennis"][..], "Tênis"),
        (&["basketball"][..], "Basquete"),
        (&["baseball"][..], "Beisebol"),
        (&["hockey"][..], "Hóquei"),
        (&["soccer"][..], "Futebol"),
        (&["football", "nfl", "ncaaf"][..], "Futebol Americano"),
        (&["ufc", "mma", "boxing"][..], "MMA & Boxe"),
        (&["cs2", "lol", "dota", "valorant"][..], "Esports"),
    ]
    .into_iter()
    .find(|(p, _)| tem(p))
    .map(|(_, nome)| nome))
}

regex_cat!(SC_POLY, r"^[a-z]{2,6}-[a-z0-9]{2,5}-[a-z0-9]{2,5}-\d{4}-\d{2}-\d{2}");
regex_cat!(SC_KALSHI, r"^kx[a-z]*(?:game|match|series)-");

/// `slugDeConfronto(slugs)`.
fn slug_de_confronto(slugs: &[String]) -> bool {
    slugs.iter().any(|s| {
        let s = s.to_lowercase();
        SC_POLY.is_match(&s) || SC_KALSHI.is_match(&s)
    })
}

regex_cat!(META_ESPORTE, r"\b(sports?|esportes?|soccer|football|basketball|baseball|hockey|tennis|golf|boxing|cricket|rugby|olympics?|olimp[ií]adas?)\b");

regex_cat!(CF_VS, r"\bvs\.?\b|\bversus\b|\s@\s");
regex_cat!(CF_AMERICANO, r"\b(quarter|touchdown|field goal|passing yards|rushing yards|receiving yards|first down|sack|interception)\b");
regex_cat!(CF_BEISEBOL, r"\b(home run|strikeout|innings?|pitcher|runs? scored|bases?|rbis?)\b");
regex_cat!(CF_BASQUETE, r"\b(rebounds?|assists?|three[ -]?pointers?|quarters?|points?)\b");
regex_cat!(CF_FUTEBOL, r"\b(goals?|gols?|both teams to score|draw no bet|corners?|cards?|clean sheet|first half|second half)\b");
regex_cat!(CF_CLUBE, r"\b(fc|fk|cf|sc|ac|afc|sk|bk|cd|united|utd|city|athletic|atl[ée]tico|dynamo|dinamo|sporting|inter|club|calcio|hotspur|rovers|albion)\b");
regex_cat!(CF_TENIS, r"\b(sets?|aces?|double faults?|tie ?break|open|masters|challenger)\b");
regex_cat!(CF_MMA, r"\b(rounds?|knockout|\bko\b|submission|by decision)\b");
regex_cat!(CF_GENERICO, r"\b(moneyline|spread|o\/u|over\/under)\b");

/// `esportePorConfronto(texto)`.
fn esporte_por_confronto(texto: &str) -> Option<&'static str> {
    if !CF_VS.is_match(texto) {
        return None;
    }
    let regras: [(&LazyLock<Regex>, &'static str); 8] = [
        (&CF_AMERICANO, "Futebol Americano"),
        (&CF_BEISEBOL, "Beisebol"),
        (&CF_BASQUETE, "Basquete"),
        (&CF_FUTEBOL, "Futebol"),
        (&CF_CLUBE, "Futebol"),
        (&CF_TENIS, "Tênis"),
        (&CF_MMA, "MMA & Boxe"),
        (&CF_GENERICO, ESPORTE_GENERICO),
    ];
    regras.iter().find(|(r, _)| r.is_match(texto)).map(|(_, nome)| *nome)
}

regex_cat!(TM_TRACOS, r"[_-]+");
regex_cat!(TM_TRANSMISSAO, r"\b(?:halftime\s+show|national\s+anthem|coin\s+toss|gatorade\s+shower|streaker|jersey\s+number)\b");
regex_cat!(TM_ANUNCIO, r"\b(?:run|air)\s+an?\s+ad\b|\b(?:commercial|advertisement)s?\b");
// `.` do JS não casa \n, \r, U+2028 nem U+2029 (o do Rust casa \r e os dois).
regex_cat!(TM_MUSICA, "\\b(?:perform|sing)\\b[^\n\r\u{2028}\u{2029}]*(?:[\"\u{201C}]|\\bsongs?\\b)|\\b(?:first|last)\\s+song\\b");
regex_cat!(TM_FINAL, r"\b(?:super\s+bowl|big\s+game)\b");
regex_cat!(TM_AUDIENCIA, r"\b(?:viewers?|watch|audience|ratings?|attend|proposes?|be\s+said|mentions?)\b");
regex_cat!(TM_COPA, "\\bcry\\b[^\n\r\u{2028}\u{2029}]*\\bworld\\s+cup\\b|\\bneymar\\s+play\\s+in\\s+the\\s+world\\s+cup\\b");
regex_cat!(TM_CONFRONTO, r"\s(?:vs\.?|versus|@)\s");

/// `temaDoMercadoProprio(d)`.
fn tema_do_mercado_proprio(d: &Value) -> Result<Option<&'static str>, Excecao> {
    let proprio = mercado_proprio(d)?;
    let partes: Vec<String> = [d.get("title"), proprio.and_then(|m| m.get("question")), d.get("slug"), proprio.and_then(|m| m.get("slug"))]
        .into_iter()
        .filter(|v| js::truthy(*v))
        .map(|v| texto_js(v.unwrap_or(&Value::Null)))
        .collect();
    let texto = TM_TRACOS.replace_all(&partes.join(" ").to_lowercase(), " ").into_owned();
    if TM_TRANSMISSAO.is_match(&texto)
        || TM_ANUNCIO.is_match(&texto)
        || TM_MUSICA.is_match(&texto)
        || (TM_FINAL.is_match(&texto) && TM_AUDIENCIA.is_match(&texto))
        || TM_COPA.is_match(&texto)
    {
        return Ok(Some("Entretenimento"));
    }
    let slugs: Vec<String> = ["slug", "eventSlug", "_eventTicker"].iter().map(|k| d.get(*k).map(texto_js).unwrap_or_else(|| "undefined".into())).collect();
    Ok(if TM_CONFRONTO.is_match(&texto) || slug_de_confronto(&slugs) { None } else { categoria_por_tema(&texto) })
}

/// `classificarAposta(d)`. `Err` quando o JS lançaria exceção.
pub fn classificar(d: &Value) -> Result<Value, Excecao> {
    if !matches!(d, Value::Object(_) | Value::Array(_)) {
        return Ok(Value::String(OUTROS.into()));
    }
    let casa = d.get("_platform").and_then(Value::as_str);
    if matches!(casa, Some("predict" | "sxbet")) {
        return Ok(d.get("_category").filter(|c| js::truthy(Some(c))).cloned().unwrap_or_else(|| Value::String(OUTROS.into())));
    }
    let s = sinais_da_aposta(d)?;
    let mut cat = match tema_do_mercado_proprio(d)? {
        Some(c) => Some(c),
        None => esporte_por_tipos_mercado(d)?,
    };
    if cat.is_none() {
        cat = s.meta.iter().find_map(|v| normalizar_esporte_v(&Value::String(v.clone())));
    }
    if cat.is_none() {
        cat = esporte_por_ticker(&s.slugs);
    }
    if cat.is_none() && (META_ESPORTE.is_match(&s.meta_texto) || slug_de_confronto(&s.slugs)) {
        cat = Some(normalizar_esporte(&s.texto).or_else(|| esporte_por_confronto(&s.texto)).unwrap_or(ESPORTE_GENERICO));
    }
    let cat = cat
        .or_else(|| normalizar_esporte(&s.texto))
        .or_else(|| categoria_por_tema(&s.texto))
        .or_else(|| esporte_por_confronto(&s.texto))
        .unwrap_or(OUTROS);
    Ok(Value::String(cat.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cat(d: Value) -> String {
        classificar(&d).unwrap().as_str().unwrap().to_string()
    }

    #[test]
    fn exemplos() {
        assert_eq!(cat(json!({ "_platform": "polymarket", "title": "Arsenal vs Chelsea", "slug": "epl-ars-che-2026-01-12-ars" })), "Futebol");
        assert_eq!(cat(json!({ "_platform": "kalshi", "title": "Yankees vs Red Sox", "_eventTicker": "KXMLBGAME-26" })), "Beisebol");
        assert_eq!(cat(json!({ "_platform": "polymarket", "title": "Will Bitcoin reach $200k?", "slug": "btc-200k" })), "Cripto");
        assert_eq!(cat(json!({ "_platform": "polymarket", "title": "Super Bowl halftime show: first song?", "sport": "NFL" })), "Entretenimento");
        assert_eq!(cat(json!({ "_platform": "polymarket", "title": "Coisa qualquer" })), OUTROS);
        assert_eq!(cat(json!({ "_platform": "polymarket", "title": "A vs B", "sport": "Sports" })), ESPORTE_GENERICO);
        assert_eq!(cat(json!({ "_platform": "predict", "_category": "Política" })), "Política");
        assert_eq!(classificar(&json!({ "_platform": "sxbet", "_category": 7 })).unwrap(), json!(7));
        assert_eq!(cat(json!("texto")), OUTROS);
        // Sufixo de clube bloqueia o tênis; sem outra pista, fica em "Outros".
        assert_eq!(cat(json!({ "_platform": "polymarket", "title": "Wimbledon final", "sport": "AFC Wimbledon" })), OUTROS);
        assert_eq!(cat(json!({ "_platform": "polymarket", "title": "Wimbledon final" })), "Tênis");
    }

    #[test]
    fn excecoes_como_no_js() {
        assert_eq!(classificar(&json!({ "_platform": "polymarket", "_sportMeta": { "markets": [null] } })), Err(Excecao));
        assert_eq!(classificar(&json!({ "_platform": "polymarket", "_sportMeta": { "markets": "abc" } })), Err(Excecao));
        assert_eq!(classificar(&json!({ "_platform": "polymarket", "_sportMeta": { "sportsMarketTypes": 5 } })), Err(Excecao));
        assert!(classificar(&json!({ "_platform": "polymarket", "_sportMeta": { "sportsMarketTypes": "tennis_x" } })).is_ok());
    }

    #[test]
    fn espacos_e_trim_do_js() {
        assert_eq!(trim_js("\u{FEFF} a \u{85}"), "a \u{85}");
        assert_eq!(texto_js(&json!([1, null, "a", [2, 3]])), "1,,a,2,3");
        assert_eq!(limpar("A_B\u{FEFF}-C"), "a b c");
    }

    /// Oráculo contra o `classificarAposta()` real: os casos e as respostas do
    /// JS vêm de `ferramentas/validacao/oraculo_categorias.mjs`
    /// (`BETTY_ORACULO_CATEGORIAS=<arquivo .jsonl>`).
    #[test]
    #[ignore]
    fn oraculo_categorias() {
        let caminho = std::env::var("BETTY_ORACULO_CATEGORIAS").expect("defina BETTY_ORACULO_CATEGORIAS");
        let texto = std::fs::read_to_string(caminho).unwrap();
        let (mut total, mut diferentes, mut exemplos) = (0usize, 0usize, Vec::new());
        for linha in texto.lines().filter(|l| !l.trim().is_empty()) {
            let caso: Value = serde_json::from_str(linha).unwrap();
            let esperado = if caso.get("erro").is_some() { Err(Excecao) } else { Ok(caso["saida"].clone()) };
            let obtido = classificar(&caso["entrada"]);
            total += 1;
            if obtido != esperado {
                diferentes += 1;
                if exemplos.len() < 20 {
                    exemplos.push(format!("{}: JS {:?} x Rust {:?}", caso["id"], esperado, obtido));
                }
            }
        }
        assert!(diferentes == 0, "{diferentes} de {total} casos diferentes; os primeiros:\n{}", exemplos.join("\n"));
        eprintln!("{total} casos iguais ao JS");
    }
}
