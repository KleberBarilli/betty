//! Leitura parcial de JSON: só um nível de um objeto ou de uma lista, com o
//! texto original de cada valor (`RawValue`). Serve para recortar ou repassar
//! JSON grande sem montar um `serde_json::Value` inteiro.
//!
//! O texto repassado é o original. Ele só sai igual ao que `js::para_vec`
//! escreveria a partir do `Value` quando o JSON já está na forma do
//! serde_json: compacto, sem chaves repetidas e sem escapes `\u`. É o caso de
//! tudo o que o Betty grava com `js::para_texto`. Quem usa este módulo confere
//! `escapes_como_o_serde` e cai no caminho com `Value` quando algo foge disso.

use std::fmt;

use serde::de::{Deserialize, Deserializer, Error, MapAccess, Visitor};
use serde_json::Value;
use serde_json::value::RawValue;

use crate::js;

/// Um objeto JSON lido só no primeiro nível: as chaves, na ordem, e o texto
/// de cada valor. Chave repetida é erro (o `Value` ficaria só com a última).
pub struct Objeto<'a>(pub Vec<(String, &'a RawValue)>);

impl<'de> Deserialize<'de> for Objeto<'de> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitante;
        impl<'de> Visitor<'de> for Visitante {
            type Value = Objeto<'de>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("um objeto JSON")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut m: A) -> Result<Objeto<'de>, A::Error> {
                let mut campos: Vec<(String, &'de RawValue)> = Vec::with_capacity(m.size_hint().unwrap_or(8));
                while let Some(campo) = m.next_entry::<String, &'de RawValue>()? {
                    campos.push(campo);
                }
                if campos.len() > 1 {
                    let mut chaves: Vec<&str> = campos.iter().map(|(k, _)| k.as_str()).collect();
                    chaves.sort_unstable();
                    if chaves.windows(2).any(|w| w[0] == w[1]) {
                        return Err(A::Error::custom("chave repetida"));
                    }
                }
                Ok(Objeto(campos))
            }
        }
        d.deserialize_map(Visitante)
    }
}

impl<'a> Objeto<'a> {
    /// `None` quando o texto não é um objeto JSON válido.
    pub fn ler(texto: &'a str) -> Option<Self> {
        serde_json::from_str(texto).ok()
    }

    pub fn get(&self, chave: &str) -> Option<&'a RawValue> {
        self.0.iter().find(|(k, _)| k == chave).map(|(_, v)| *v)
    }
}

/// Os itens de uma lista JSON, cada um com o texto original. `None` quando
/// o valor não é uma lista.
pub fn lista(r: &RawValue) -> Option<Vec<&RawValue>> {
    serde_json::from_str(r.get()).ok()
}

/// O valor decodificado. Para campos pequenos.
pub fn valor(r: &RawValue) -> Value {
    serde_json::from_str(r.get()).unwrap_or(Value::Null)
}

fn primeiro(r: &RawValue) -> u8 {
    r.get().trim_start().as_bytes().first().copied().unwrap_or(b' ')
}

pub fn e_objeto(r: &RawValue) -> bool {
    primeiro(r) == b'{'
}

pub fn e_lista(r: &RawValue) -> bool {
    primeiro(r) == b'['
}

/// `js::truthy` sem decodificar objetos e listas (sempre verdadeiros).
pub fn verdadeiro(r: Option<&RawValue>) -> bool {
    match r {
        None => false,
        Some(r) if e_objeto(r) || e_lista(r) => true,
        Some(r) => js::truthy(Some(&valor(r))),
    }
}

/// Sem `\u` nem `\/` no texto: os escapes que sobram (`\"`, `\\`, `\n`...) o
/// serde_json escreve do mesmo jeito que leu.
pub fn escapes_como_o_serde(texto: &str) -> bool {
    !texto.contains("\\u") && !texto.contains("\\/")
}

/// Escreve `{"chave":valor,...}` com as chaves escapadas como o serde_json e
/// os valores já em texto JSON.
pub fn escrever_objeto<'b>(saida: &mut Vec<u8>, campos: impl IntoIterator<Item = (&'b str, &'b [u8])>) {
    saida.push(b'{');
    for (i, (chave, valor)) in campos.into_iter().enumerate() {
        if i > 0 {
            saida.push(b',');
        }
        saida.extend(js::para_vec(chave));
        saida.push(b':');
        saida.extend_from_slice(valor);
    }
    saida.push(b'}');
}

/// Escreve `[item,...]` com os itens já em texto JSON.
pub fn escrever_lista<'b>(saida: &mut Vec<u8>, itens: impl IntoIterator<Item = &'b [u8]>) {
    saida.push(b'[');
    for (i, item) in itens.into_iter().enumerate() {
        if i > 0 {
            saida.push(b',');
        }
        saida.extend_from_slice(item);
    }
    saida.push(b']');
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn objeto_na_ordem_e_texto_original() {
        let texto = r#"{"b":1.50,"a":{"x":[1,2]},"c":"t\"x"}"#;
        let o = Objeto::ler(texto).unwrap();
        let chaves: Vec<&str> = o.0.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(chaves, ["b", "a", "c"]);
        assert_eq!(o.get("b").unwrap().get(), "1.50", "o número sai como estava");
        assert_eq!(o.get("a").unwrap().get(), r#"{"x":[1,2]}"#);
        assert!(Objeto::ler(r#"{"a":1,"a":2}"#).is_none(), "chave repetida");
        assert!(Objeto::ler("[1]").is_none());
        let mut saida = Vec::new();
        escrever_objeto(&mut saida, o.0.iter().map(|(k, v)| (k.as_str(), v.get().as_bytes())));
        assert_eq!(String::from_utf8(saida).unwrap(), texto);
    }

    #[test]
    fn verdadeiro_como_no_js() {
        let v = |t: &str| verdadeiro(Some(&RawValue::from_string(t.into()).unwrap()));
        for (t, esperado) in [("{}", true), ("[]", true), ("0", false), ("\"\"", false), ("null", false), ("\"a\"", true), ("1", true), ("false", false)] {
            assert_eq!(v(t), esperado, "{t}");
        }
        assert!(!verdadeiro(None));
        let r = RawValue::from_string("[1,{\"a\":2}]".into()).unwrap();
        assert_eq!(lista(&r).unwrap().iter().map(|x| x.get()).collect::<Vec<_>>(), ["1", "{\"a\":2}"]);
        assert_eq!(valor(&r), json!([1, {"a": 2}]));
    }
}
