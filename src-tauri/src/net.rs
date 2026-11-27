//! Cliente HTTP compartilhado e assinatura RSA-PSS (Kalshi).

use std::sync::LazyLock;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use serde_json::Value;

/// Cliente único (pool de conexões compartilhado). Sem proxy do sistema.
pub static CLIENTE: LazyLock<reqwest::Client> = LazyLock::new(|| {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(15))
        .build()
        .expect("cliente HTTP")
});

/// Erro de uma resposta HTTP não-2xx, com o status para a rota repassar.
#[derive(Debug)]
pub struct ErroHttp {
    pub status: u16,
    pub corpo: Value,
}

impl std::fmt::Display for ErroHttp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HTTP {}", self.status)
    }
}

impl std::error::Error for ErroHttp {}

/// Status HTTP de um erro, quando ele veio de uma resposta da API.
pub fn status_do_erro(e: &anyhow::Error) -> Option<u16> {
    e.downcast_ref::<ErroHttp>().map(|h| h.status)
}

/// GET que devolve o JSON só quando a resposta é 2xx (`r.ok ? r.json() : null`).
///
/// Diferença deliberada em relação ao Node: 429/503 (limite de requisições)
/// são repetidos até 3 vezes, respeitando `Retry-After`. No Node, um 429 numa
/// consulta de metadados virava "sem dados" e o evento incompleto ia para o
/// cache por 6 horas (times invertidos, sem modalidade ou horário oficial).
pub async fn get_json(url: &str, prazo: Duration) -> Result<Option<Value>> {
    for tentativa in 1..=3u64 {
        let r = CLIENTE.get(url).header("accept", "application/json").timeout(prazo).send().await?;
        let status = r.status().as_u16();
        if (status == 429 || status == 503) && tentativa < 3 {
            let espera = r
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<f64>().ok())
                .map_or(Duration::from_millis(700 * tentativa), |s| Duration::from_secs_f64(s.clamp(0.2, 5.0)));
            tokio::time::sleep(espera).await;
            continue;
        }
        if !r.status().is_success() {
            return Ok(None);
        }
        let texto = r.text().await?;
        return Ok(Some(serde_json::from_str(&texto)?));
    }
    Ok(None)
}

/// Chave privada RSA lida de um PEM (PKCS#1 "RSA PRIVATE KEY" ou PKCS#8 "PRIVATE KEY").
pub fn chave_rsa(pem: &str) -> Result<ring::rsa::KeyPair> {
    let texto = pem.trim();
    let pkcs1 = texto.contains("BEGIN RSA PRIVATE KEY");
    if !pkcs1 && !texto.contains("BEGIN PRIVATE KEY") {
        bail!("PEM sem chave privada RSA reconhecida");
    }
    let corpo: String = texto.lines().filter(|l| !l.starts_with("-----")).map(str::trim).collect();
    let der = zeroize::Zeroizing::new(base64::engine::general_purpose::STANDARD.decode(corpo).context("PEM com base64 inválido")?);
    let par = if pkcs1 { ring::rsa::KeyPair::from_der(&der) } else { ring::rsa::KeyPair::from_pkcs8(&der) };
    par.map_err(|e| anyhow!("chave RSA rejeitada: {e}"))
}

/// Assinatura RSA-PSS SHA-256 (salt de 32 bytes) em base64, como a Kalshi exige.
pub fn assinar_pss(chave: &ring::rsa::KeyPair, mensagem: &[u8]) -> Result<String> {
    let mut assinatura = vec![0u8; chave.public().modulus_len()];
    chave
        .sign(&ring::signature::RSA_PSS_SHA256, &ring::rand::SystemRandom::new(), mensagem, &mut assinatura)
        .map_err(|_| anyhow!("falha ao assinar"))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(assinatura))
}

/// `encodeURIComponent()`.
pub fn codificar(s: &str) -> String {
    let mut saida = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            saida.push(b as char);
        } else {
            saida.push_str(&format!("%{b:02X}"));
        }
    }
    saida
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codifica_como_o_js() {
        assert_eq!(codificar("a,b c/é"), "a%2Cb%20c%2F%C3%A9");
        assert_eq!(codificar("KX-1_a.b~(x)"), "KX-1_a.b~(x)");
    }

    #[test]
    fn assina_e_verifica_pss() {
        // A chave é gerada na hora (2048 bits, PKCS#8): nenhuma chave privada fica no repositório.
        use rsa::pkcs8::EncodePrivateKey;
        let gerada = rsa::RsaPrivateKey::new(&mut rand::thread_rng(), 2048).unwrap();
        let pem = gerada.to_pkcs8_pem(rsa::pkcs8::LineEnding::LF).unwrap();
        let chave = chave_rsa(&pem).unwrap();
        let assinatura = assinar_pss(&chave, b"1700000000000GET/trade-api/v2/portfolio/balance").unwrap();
        let bytes = base64::engine::general_purpose::STANDARD.decode(assinatura).unwrap();
        let publica = ring::signature::UnparsedPublicKey::new(&ring::signature::RSA_PSS_2048_8192_SHA256, chave.public().as_ref());
        publica.verify(b"1700000000000GET/trade-api/v2/portfolio/balance", &bytes).unwrap();
        assert!(chave_rsa("-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----").is_err());
    }
}
