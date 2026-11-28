//! Conversões com a mesma semântica do JavaScript do `server.js`. As validações
//! do backend antigo dependem delas (`Number(x) > 0`, `String(x)`, ...), e o
//! JSON gravado no banco precisa sair igual ao do Node.

use serde_json::Value;

/// `Number(valor)`; `None` representa `undefined`.
pub fn number(valor: Option<&Value>) -> f64 {
    match valor {
        None => f64::NAN,
        Some(Value::Null) => 0.0,
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(s)) => texto_para_numero(s),
        // Number([]) = 0, Number([x]) = Number(String(x)), demais = NaN.
        Some(Value::Array(a)) => match a.as_slice() {
            [] => 0.0,
            [unico] => match unico {
                Value::Array(_) | Value::Object(_) => f64::NAN,
                Value::Null => 0.0,
                outro => texto_para_numero(&string(Some(outro))),
            },
            _ => f64::NAN,
        },
        Some(Value::Object(_)) => f64::NAN,
    }
}

fn texto_para_numero(s: &str) -> f64 {
    let t = s.trim();
    if t.is_empty() {
        return 0.0;
    }
    match t {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    for (prefixo, base) in [("0x", 16), ("0X", 16), ("0o", 8), ("0O", 8), ("0b", 2), ("0B", 2)] {
        if let Some(resto) = t.strip_prefix(prefixo) {
            return u64::from_str_radix(resto, base).map(|n| n as f64).unwrap_or(f64::NAN);
        }
    }
    // O Rust aceita "inf"/"nan" e o JavaScript não.
    if t.bytes().any(|b| b.is_ascii_alphabetic() && b != b'e' && b != b'E') {
        return f64::NAN;
    }
    t.parse::<f64>().unwrap_or(f64::NAN)
}

/// `String(valor)` para os tipos simples.
pub fn string(valor: Option<&Value>) -> String {
    match valor {
        None => "undefined".into(),
        Some(Value::Null) => "null".into(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => match (n.as_i64(), n.as_u64()) {
            (Some(i), _) => i.to_string(),
            (_, Some(u)) => u.to_string(),
            _ => numero_texto(n.as_f64().unwrap_or(f64::NAN)).unwrap_or_else(|| "NaN".into()),
        },
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(_)) => String::new(),
        Some(Value::Object(_)) => "[object Object]".into(),
    }
}

/// `Number.prototype.toString()` (ECMAScript Number::toString): mesmos
/// dígitos mínimos e as mesmas regras de quando usar expoente.
pub fn numero_texto(n: f64) -> Option<String> {
    if !n.is_finite() {
        return None;
    }
    if n == 0.0 {
        return Some("0".into());
    }
    // `{:e}` do Rust dá os dígitos mínimos que fazem ida e volta, como o JS.
    let cientifico = format!("{:e}", n.abs());
    let (mantissa, expoente) = cientifico.split_once('e')?;
    let digitos: String = mantissa.chars().filter(|c| *c != '.').collect();
    let k = digitos.len() as i32;
    let pos = expoente.parse::<i32>().ok()? + 1;
    let corpo = if k <= pos && pos <= 21 {
        format!("{digitos}{}", "0".repeat((pos - k) as usize))
    } else if 0 < pos && pos <= 21 {
        format!("{}.{}", &digitos[..pos as usize], &digitos[pos as usize..])
    } else if -6 < pos && pos <= 0 {
        format!("0.{}{digitos}", "0".repeat((-pos) as usize))
    } else {
        let e = pos - 1;
        let resto = if k > 1 { format!(".{}", &digitos[1..]) } else { String::new() };
        format!("{}{resto}e{}{}", &digitos[..1], if e >= 0 { "+" } else { "-" }, e.abs())
    };
    Some(if n < 0.0 { format!("-{corpo}") } else { corpo })
}

/// Número como valor JSON, com o texto que o `JSON.stringify` escreveria
/// (com `arbitrary_precision` o texto fica guardado no próprio número).
pub fn numero_json(n: f64) -> Value {
    if !n.is_finite() {
        Value::Null
    } else if n.fract() == 0.0 && n.abs() < 9_007_199_254_740_992.0 {
        Value::from(n as i64)
    } else {
        numero_texto(n).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null)
    }
}

/// Formatador do serde_json que escreve números com as regras do
/// `JSON.stringify` (0.000001, 1e-7, 1e+21...).
pub struct FormatoJs;

impl serde_json::ser::Formatter for FormatoJs {
    fn write_f64<W: ?Sized + std::io::Write>(&mut self, w: &mut W, v: f64) -> std::io::Result<()> {
        w.write_all(numero_texto(v).as_deref().unwrap_or("null").as_bytes())
    }

    fn write_f32<W: ?Sized + std::io::Write>(&mut self, w: &mut W, v: f32) -> std::io::Result<()> {
        self.write_f64(w, f64::from(v))
    }
}

/// `JSON.stringify(v)` em bytes.
pub fn para_vec<T: serde::Serialize + ?Sized>(v: &T) -> Vec<u8> {
    let mut buf = Vec::with_capacity(256);
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, FormatoJs);
    v.serialize(&mut ser).expect("serialização JSON");
    buf
}

/// `para_vec(v).len()` sem guardar o texto: conta os bytes que o mesmo
/// serializador escreveria. Serve para medir listas grandes sem alocar uma
/// cópia inteira delas.
pub fn tamanho_json<T: serde::Serialize + ?Sized>(v: &T) -> usize {
    struct Contador(usize);
    impl std::io::Write for Contador {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0 += b.len();
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut contador = Contador(0);
    let mut ser = serde_json::Serializer::with_formatter(&mut contador, FormatoJs);
    v.serialize(&mut ser).expect("serialização JSON");
    contador.0
}

/// `JSON.stringify(v)`.
pub fn para_texto<T: serde::Serialize + ?Sized>(v: &T) -> String {
    String::from_utf8(para_vec(v)).expect("JSON é UTF-8")
}

/// Valor bruto (texto JSON) com a mesma formatação.
pub fn bruto<T: serde::Serialize + ?Sized>(v: &T) -> Box<serde_json::value::RawValue> {
    serde_json::value::RawValue::from_string(para_texto(v)).expect("JSON válido")
}

/// Regex escrita na sintaxe do JavaScript: lá `\b` e `\d` são ASCII; no crate
/// `regex` seriam Unicode.
pub fn re(padrao: &str) -> regex::Regex {
    let traduzido = padrao.replace(r"\b", r"(?-u:\b)").replace(r"\d", "[0-9]");
    regex::Regex::new(&traduzido).unwrap_or_else(|e| panic!("regex inválida {padrao}: {e}"))
}

/// Declara uma regex estática compilada na primeira utilização.
#[macro_export]
macro_rules! regex_js {
    ($nome:ident, $padrao:expr) => {
        static $nome: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| $crate::js::re($padrao));
    };
}

/// `str.slice(0, n)` do JavaScript, que conta unidades UTF-16.
pub fn fatia_utf16(s: &str, n: usize) -> &str {
    let mut unidades = 0;
    for (i, c) in s.char_indices() {
        unidades += c.len_utf16();
        if unidades > n {
            return &s[..i];
        }
    }
    s
}

/// `Date.parse()` para os formatos que as APIs usam (ISO 8601, data simples,
/// data e hora sem fuso = horário local, RFC 2822). `None` = NaN.
pub fn data_ms(texto: &str) -> Option<f64> {
    use chrono::{DateTime, NaiveDate, TimeZone};
    static DATA: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"(?i)^(\d{4})-(\d{1,2})-(\d{1,2})(?:[T ](\d{2}):(\d{2})(?::(\d{2})(?:\.(\d+))?)?)?\s*(Z|[+-]\d{2}(?::?\d{2})?)?$",
        )
        .expect("regex de data")
    });
    let t = texto.trim();
    if t.is_empty() {
        return None;
    }
    if let Some(c) = DATA.captures(t) {
        let n = |i: usize| c.get(i).map_or(Some(0), |m| m.as_str().parse::<u32>().ok());
        let data = NaiveDate::from_ymd_opt(c[1].parse().ok()?, n(2)?, n(3)?)?;
        // Fração: só os milissegundos contam (o V8 trunca o resto).
        let ms = c.get(7).map_or(0, |m| {
            let f: String = m.as_str().chars().chain(std::iter::repeat('0')).take(3).collect();
            f.parse::<u32>().unwrap_or(0)
        });
        let local = data.and_hms_milli_opt(n(4)?, n(5)?, n(6)?, ms)?;
        let tem_hora = c.get(4).is_some();
        return match c.get(8).map(|m| m.as_str()) {
            Some(z) if z.eq_ignore_ascii_case("z") => Some(local.and_utc().timestamp_millis() as f64),
            Some(fuso) => {
                let sinal = if fuso.starts_with('-') { -1 } else { 1 };
                let digitos: String = fuso[1..].chars().filter(char::is_ascii_digit).collect();
                let horas: i32 = digitos.get(..2)?.parse().ok()?;
                let minutos: i32 = digitos.get(2..).filter(|s| !s.is_empty()).map_or(Some(0), |s| s.parse().ok())?;
                let deslocamento = chrono::FixedOffset::east_opt(sinal * (horas * 3600 + minutos * 60))?;
                deslocamento.from_local_datetime(&local).single().map(|d| d.timestamp_millis() as f64)
            }
            // Data ISO pura (AAAA-MM-DD) é UTC; com hora ou fora do formato estrito, horário local.
            None if !tem_hora && c[2].len() == 2 && c[3].len() == 2 => Some(local.and_utc().timestamp_millis() as f64),
            None => chrono::Local.from_local_datetime(&local).earliest().map(|d| d.timestamp_millis() as f64),
        };
    }
    DateTime::parse_from_rfc2822(t).ok().map(|d| d.timestamp_millis() as f64)
}

/// `new Date(ms).toISOString()`.
pub fn iso(ms: f64) -> Option<String> {
    use chrono::TimeZone;
    if !ms.is_finite() || ms.abs() > 8.64e15 {
        return None;
    }
    chrono::Utc
        .timestamp_millis_opt(ms.trunc() as i64)
        .single()
        .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

/// `/^\d{4}-\d{2}-\d{2}$/`.
pub fn e_data_ymd(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
}

/// Truthiness do JavaScript.
pub fn truthy(valor: Option<&Value>) -> bool {
    match valor {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(_)) | Some(Value::Object(_)) => true,
    }
}

/// `String(valor)` completo: listas viram `"a,b"` (com `null` vazio), como no
/// `Array.prototype.join`, e números saem como o `f64` que o `JSON.parse` do
/// JS teria lido (acima de 2^53 perdem dígitos; fora do `f64`, `Infinity`).
pub fn texto(valor: Option<&Value>) -> String {
    match valor {
        Some(Value::Array(itens)) => itens.iter().map(|i| if i.is_null() { String::new() } else { texto(Some(i)) }).collect::<Vec<_>>().join(","),
        Some(Value::Number(n)) => match n.as_f64() {
            Some(f) if f == f64::INFINITY => "Infinity".into(),
            Some(f) if f == f64::NEG_INFINITY => "-Infinity".into(),
            Some(f) => numero_texto(f).unwrap_or_else(|| "NaN".into()),
            // `as_f64` não devolve infinito: o texto guardado (`1e400`) diz o sinal.
            None => match n.to_string().parse::<f64>() {
                Ok(f) if f == f64::INFINITY => "Infinity".into(),
                Ok(f) if f == f64::NEG_INFINITY => "-Infinity".into(),
                _ => "NaN".into(),
            },
        },
        outro => string(outro),
    }
}

/// `obj[chave]` do JavaScript sobre um valor JSON, sem o protótipo: campo de
/// objeto, índice ou `length` de lista e de texto (unidades UTF-16; metade de
/// um par substituto fica `None`).
pub fn propriedade(obj: &Value, chave: &str) -> Option<Value> {
    let indice = || -> Option<usize> {
        let canonico = chave == "0" || (!chave.starts_with('0') && !chave.is_empty() && chave.bytes().all(|b| b.is_ascii_digit()));
        chave.parse::<u64>().ok().filter(|n| canonico && *n < 4_294_967_295).map(|n| n as usize)
    };
    match obj {
        Value::Object(m) => m.get(chave).cloned(),
        Value::Array(a) if chave == "length" => Some(Value::from(a.len())),
        Value::Array(a) => indice().and_then(|i| a.get(i).cloned()),
        Value::String(s) if chave == "length" => Some(Value::from(tamanho_utf16(s))),
        Value::String(s) => {
            let unidade = s.encode_utf16().nth(indice()?)?;
            char::from_u32(u32::from(unidade)).map(|c| Value::String(c.to_string()))
        }
        _ => None,
    }
}

/// `String(v || "")`.
pub fn texto_ou_vazio(valor: Option<&Value>) -> String {
    if truthy(valor) { texto(valor) } else { String::new() }
}

/// Caracteres do `\s` e do `trim()` do JavaScript: incluem U+FEFF e não
/// incluem U+0085 (o contrário do `char::is_whitespace`).
pub fn espaco_js(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'..='\u{200A}' | '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' | '\u{3000}' | '\u{FEFF}'
    )
}

/// `String.prototype.trim()`.
pub fn trim_js(s: &str) -> &str {
    s.trim_matches(espaco_js)
}

/// `s.replace(/\s+/g, " ").trim()`.
pub fn limpar(s: &str) -> String {
    s.split(espaco_js).filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" ")
}

/// Unidades UTF-16 de um texto (o `length` do JavaScript).
pub fn tamanho_utf16(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// `s.slice(n)` com `n` em unidades UTF-16 (um par substituto partido ao
/// meio fica inteiro do lado de cá).
pub fn resto_utf16(s: &str, n: usize) -> &str {
    &s[fatia_utf16(s, n).len()..]
}

/// Corpo da classe do `\s` do JavaScript.
const CLASSE_ESPACO: &str = r"\t\n\x0B\x0C\r \x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}";

/// Regex do JavaScript (sem a flag `u`) com a semântica dele: `\s`, `\S`, `.`,
/// `\d`, `\w` e `\b` como no JS, e `[`, `&`, `~` e `--` literais dentro de
/// classes (no crate `regex` são operadores). O `(?i)` continua o do Rust: ele
/// também junta `ſ` a `s` e o sinal de kelvin a `k`, o que o JS sem `u` não faz.
pub fn re_exata(padrao: &str) -> regex::Regex {
    regex::Regex::new(&traduzir_regex(padrao)).unwrap_or_else(|e| panic!("regex inválida {padrao}: {e}"))
}

fn traduzir_regex(padrao: &str) -> String {
    let mut saida = String::with_capacity(padrao.len() * 2);
    let mut letras = padrao.chars().peekable();
    let mut em_classe = false;
    while let Some(c) = letras.next() {
        match c {
            '\\' => {
                let Some(e) = letras.next() else {
                    saida.push('\\');
                    break;
                };
                let classe = match e {
                    's' => Some((false, CLASSE_ESPACO)),
                    'S' => Some((true, CLASSE_ESPACO)),
                    'd' => Some((false, "0-9")),
                    'D' => Some((true, "0-9")),
                    'w' => Some((false, "A-Za-z0-9_")),
                    'W' => Some((true, "A-Za-z0-9_")),
                    _ => None,
                };
                match (classe, e) {
                    (Some((false, corpo)), _) if em_classe => {
                        saida.push_str(corpo);
                        // `[\s-x]`: no JS o hífen depois de uma classe é literal.
                        if letras.peek() == Some(&'-') {
                            letras.next();
                            saida.push_str(r"\-");
                        }
                    }
                    (Some((negada, corpo)), _) => {
                        saida.push_str(if negada { "[^" } else { "[" });
                        saida.push_str(corpo);
                        saida.push(']');
                    }
                    (None, 'b') if em_classe => saida.push_str(r"\x08"),
                    (None, 'b') => saida.push_str(r"(?-u:\b)"),
                    (None, '/') => saida.push('/'),
                    (None, outro) => {
                        saida.push('\\');
                        saida.push(outro);
                    }
                }
            }
            '[' if em_classe => saida.push_str(r"\["),
            '[' => {
                em_classe = true;
                saida.push('[');
                if letras.peek() == Some(&'^') {
                    letras.next();
                    saida.push('^');
                }
            }
            ']' if em_classe => {
                em_classe = false;
                saida.push(']');
            }
            '&' | '~' if em_classe => {
                saida.push('\\');
                saida.push(c);
            }
            '-' if em_classe && letras.peek() == Some(&'-') => saida.push_str(r"\-"),
            '.' if !em_classe => saida.push_str(r"[^\n\r\x{2028}\x{2029}]"),
            outro => saida.push(outro),
        }
    }
    saida
}

/// Declara uma regex estática do JavaScript (`re_exata`), compilada no primeiro uso.
#[macro_export]
macro_rules! regex_exata {
    ($nome:ident, $padrao:expr) => {
        static $nome: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| $crate::js::re_exata($padrao));
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tamanho_igual_ao_do_texto() {
        let valores = [
            json!(null),
            json!([]),
            json!({ "a": 1.5, "b": "ação \"x\" \n", "c": [1, 2, { "d": null }], "e": numero_json(1e21), "f": numero_json(0.000001) }),
            json!((0..500).map(|i| json!({ "i": i, "t": format!("texto {i}"), "x": numero_json(f64::from(i) / 7.0) })).collect::<Vec<_>>()),
        ];
        for v in &valores {
            assert_eq!(tamanho_json(v), para_vec(v).len(), "{v}");
        }
    }

    #[test]
    fn number_segue_o_javascript() {
        assert!(number(None).is_nan());
        assert_eq!(number(Some(&json!(null))), 0.0);
        assert_eq!(number(Some(&json!(""))), 0.0);
        assert_eq!(number(Some(&json!("  2.5 "))), 2.5);
        assert_eq!(number(Some(&json!("1e2"))), 100.0);
        assert_eq!(number(Some(&json!("0x10"))), 16.0);
        assert!(number(Some(&json!("inf"))).is_nan());
        assert!(number(Some(&json!("abc"))).is_nan());
        assert_eq!(number(Some(&json!("Infinity"))), f64::INFINITY);
        assert_eq!(number(Some(&json!(true))), 1.0);
        assert_eq!(number(Some(&json!([]))), 0.0);
        assert_eq!(number(Some(&json!(["7"]))), 7.0);
        assert!(number(Some(&json!({}))).is_nan());
    }

    #[test]
    fn json_numerico_igual_ao_node() {
        let casos = [
            (1.0, "1"),
            (1.5, "1.5"),
            (-0.0, "0"),
            (0.56, "0.56"),
            (0.000001, "0.000001"),
            (0.0000001, "1e-7"),
            (0.00001234, "0.00001234"),
            (1e21, "1e+21"),
            (123456789012345680000.0, "123456789012345680000"),
            (1.0 / 3.0, "0.3333333333333333"),
            (-2.5e-10, "-2.5e-10"),
            (1795714000000.0, "1795714000000"),
            (0.1 + 0.2, "0.30000000000000004"),
        ];
        for (n, esperado) in casos {
            assert_eq!(para_texto(&numero_json(n)), esperado, "{n}");
        }
        // Texto do Node relido e reescrito sem mudar.
        let original = r#"{"a":0.000001,"b":1e-7,"c":1795714000000,"d":0.30000000000000004,"e":1e+21,"f":100000000000000000000,"g":0.9199999999999999}"#;
        assert_eq!(para_texto(&serde_json::from_str::<Value>(original).unwrap()), original);
        assert_eq!(numero_json(f64::NAN), Value::Null);
    }

    #[test]
    fn datas_como_no_javascript() {
        // Valores de referência calculados com Date.parse do Node 24.
        assert_eq!(data_ms("2026-11-20T23:00:00Z"), Some(1795215600000.0));
        assert_eq!(data_ms("2026-11-20T23:00:00.123456Z"), Some(1795215600123.0));
        assert_eq!(data_ms("2026-11-20"), Some(1795132800000.0));
        assert_eq!(data_ms(""), None);
        assert_eq!(data_ms("abc"), None);
        assert_eq!(data_ms("2026-09-15 23:49:18+00"), Some(1789516158000.0));
        assert_eq!(data_ms("2026-09-15T23:49:18+0300"), Some(1789505358000.0));
        assert_eq!(data_ms("2026-09-15T23:49:18+03:00"), Some(1789505358000.0));
        assert_eq!(data_ms("2026-09-15 23:49:18Z"), Some(1789516158000.0));
        assert_eq!(data_ms("2026-09-15T23:49:18.5Z"), Some(1789516158500.0));
        assert_eq!(data_ms("2026-09-15T23:49:18.123456+00:00"), Some(1789516158123.0));
        assert_eq!(data_ms("2026-09-15 23:49:18.123+00"), Some(1789516158123.0));
        assert_eq!(data_ms("2026-09-15T23:49Z"), Some(1789516140000.0));
        assert_eq!(data_ms("2026-09-15 23:49:18 +00:00"), Some(1789516158000.0));
        assert_eq!(data_ms("Tue, 15 Sep 2026 23:49:18 GMT"), Some(1789516158000.0));
        assert_eq!(iso(1795215600123.0).as_deref(), Some("2026-11-20T23:00:00.123Z"));
        assert_eq!(fatia_utf16("ação😀x", 4), "ação");
        assert_eq!(fatia_utf16("ação😀x", 5), "ação");
        assert_eq!(fatia_utf16("ação😀x", 6), "ação😀");
    }

    #[test]
    fn datas_ymd() {
        assert!(e_data_ymd("2026-11-26"));
        assert!(!e_data_ymd("2026-1-26"));
        assert!(!e_data_ymd("2026/11/26"));
        assert!(!e_data_ymd("2026-11-26T00"));
    }
}
