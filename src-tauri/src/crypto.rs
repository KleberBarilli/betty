//! Cifra das credenciais, compatível byte a byte com o `server.js`:
//! AES-256-GCM, nonce de 12 bytes e saída `base64url(nonce | tag | ciphertext)`.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use anyhow::{Context, Result, anyhow, ensure};
use base64::Engine;
use base64::alphabet;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use zeroize::Zeroizing;

const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;

/// O Node gera base64url sem padding e aceita os dois formatos na leitura.
const B64: GeneralPurpose = GeneralPurpose::new(
    &alphabet::URL_SAFE,
    GeneralPurposeConfig::new()
        .with_encode_padding(false)
        .with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// Chave mestra de 32 bytes. É apagada da memória ao sair de escopo e nunca
/// aparece em logs.
pub struct MasterKey(Zeroizing<[u8; 32]>);

impl std::fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MasterKey(***)")
    }
}

impl MasterKey {
    pub fn from_hex(texto: &str) -> Result<Self> {
        let texto = texto.trim();
        ensure!(
            texto.len() == 64 && texto.bytes().all(|b| b.is_ascii_hexdigit()),
            "a chave mestra precisa ter 64 caracteres hexadecimais"
        );
        let mut bytes = Zeroizing::new([0u8; 32]);
        hex::decode_to_slice(texto, &mut bytes[..]).context("chave mestra inválida")?;
        Ok(Self(bytes))
    }

    pub fn generate() -> Result<Self> {
        let mut bytes = Zeroizing::new([0u8; 32]);
        getrandom::fill(&mut bytes[..]).map_err(|e| anyhow!("gerador aleatório indisponível: {e}"))?;
        Ok(Self(bytes))
    }

    pub fn to_hex(&self) -> Zeroizing<String> {
        Zeroizing::new(hex::encode(&self.0[..]))
    }

    fn cipher(&self) -> Aes256Gcm {
        Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&self.0[..]))
    }

    pub fn encrypt(&self, texto: &str) -> Result<String> {
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce).map_err(|e| anyhow!("gerador aleatório indisponível: {e}"))?;
        // O crate devolve `ciphertext | tag`; o formato do Betty é `nonce | tag | ciphertext`.
        let saida = self
            .cipher()
            .encrypt(Nonce::from_slice(&nonce), texto.as_bytes())
            .map_err(|_| anyhow!("falha ao cifrar"))?;
        let (cifrado, tag) = saida.split_at(saida.len() - TAG_LEN);
        let mut bruto = Vec::with_capacity(NONCE_LEN + TAG_LEN + cifrado.len());
        bruto.extend_from_slice(&nonce);
        bruto.extend_from_slice(tag);
        bruto.extend_from_slice(cifrado);
        Ok(B64.encode(bruto))
    }

    /// Igual ao `dec()` do Node: valor vazio devolve texto vazio.
    pub fn decrypt(&self, valor: &str) -> Result<Zeroizing<String>> {
        if valor.is_empty() {
            return Ok(Zeroizing::new(String::new()));
        }
        let bruto = Zeroizing::new(B64.decode(valor).context("dado cifrado mal formatado")?);
        ensure!(bruto.len() >= NONCE_LEN + TAG_LEN, "dado cifrado curto demais");
        let (nonce, resto) = bruto.split_at(NONCE_LEN);
        let (tag, cifrado) = resto.split_at(TAG_LEN);
        let mut junto = Zeroizing::new(Vec::with_capacity(cifrado.len() + TAG_LEN));
        junto.extend_from_slice(cifrado);
        junto.extend_from_slice(tag);
        let texto = self
            .cipher()
            .decrypt(Nonce::from_slice(nonce), junto.as_slice())
            .map_err(|_| anyhow!("não foi possível decifrar: chave incorreta ou dado alterado"))?;
        Ok(Zeroizing::new(String::from_utf8(texto).context("dado decifrado não é UTF-8")?))
    }

    /// `crypto.createHmac('sha256', MASTER_KEY).update(texto).digest('hex')`.
    pub fn hmac_hex(&self, texto: &str) -> String {
        let chave = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &self.0[..]);
        hex::encode(ring::hmac::sign(&chave, texto.as_bytes()).as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Chave fictícia (32 bytes 0x11). Os vetores abaixo foram gerados pelo
    /// mesmo código de `enc()` do `server.js`, rodando no Node 24.
    fn chave_teste() -> MasterKey {
        MasterKey::from_hex(&"11".repeat(32)).unwrap()
    }

    #[test]
    fn decifra_o_que_o_node_cifrou() {
        let k = chave_teste();
        assert_eq!(
            k.decrypt("QOWO-MwUgb8OaYCt6f4v5Os6_08SyxD2yvR7pXt9oEZpH66zJvJ2qX8kbwDXIoADPzjc86P1J1MDoiE9usJVX5qROIqz5Q")
                .unwrap()
                .as_str(),
            "0x1111111111111111111111111111111111111111"
        );
        assert_eq!(
            k.decrypt("HV1vN5Lowsf1kVoOP6RZjcVmhd5xYAOCrGcPVvoOa8imZxe-vz9B2N30vY3fkGG9jZRJcPbz3RPXClW8C2al7ZKONfDwiWnCACmoh4Hqrw")
                .unwrap()
                .as_str(),
            r#"{"wallet":"0xabc","apiKey":"chave-ficticia-ação"}"#
        );
        assert_eq!(k.decrypt("uNG1rJcYJXzoh_kmqw0gbx7SehMArLSGqbea8Q").unwrap().as_str(), "");
        assert_eq!(k.decrypt("").unwrap().as_str(), "");
    }

    #[test]
    fn ida_e_volta_e_nonce_unico() {
        let k = chave_teste();
        let a = k.encrypt("segredo ç").unwrap();
        let b = k.encrypt("segredo ç").unwrap();
        assert_ne!(a, b, "cada cifra precisa de nonce novo");
        assert_eq!(k.decrypt(&a).unwrap().as_str(), "segredo ç");
        assert!(!a.contains('='), "base64url sem padding, como no Node");
    }

    #[test]
    fn rejeita_chave_errada_e_dado_alterado() {
        let k = chave_teste();
        let outra = MasterKey::from_hex(&"22".repeat(32)).unwrap();
        let c = k.encrypt("x").unwrap();
        assert!(outra.decrypt(&c).is_err());
        let mut alterado = c.into_bytes();
        let ultimo = alterado.len() - 1;
        alterado[ultimo] = if alterado[ultimo] == b'A' { b'B' } else { b'A' };
        assert!(k.decrypt(std::str::from_utf8(&alterado).unwrap()).is_err());
    }

    #[test]
    fn valida_formato_da_chave() {
        assert!(MasterKey::from_hex("abc").is_err());
        assert!(MasterKey::from_hex(&"zz".repeat(32)).is_err());
        assert_eq!(format!("{:?}", chave_teste()), "MasterKey(***)");
    }
}
