//! Chave mestra no Cofre de Credenciais do Windows.
//!
//! No Electron a chave ficava em texto puro no `local-config.json`, ao lado do
//! banco: quem copiasse a pasta levava as duas coisas. Aqui ela fica no cofre
//! do usuário do Windows, protegido pelo login dele.

use std::path::Path;

use anyhow::{Context, Result};

use crate::crypto::MasterKey;

const CONTA: &str = "master-key";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrigemChave {
    /// Já estava no cofre.
    Cofre,
    /// Veio do `local-config.json` do Electron e foi copiada para o cofre.
    ImportadaDoElectron,
    /// Primeira execução sem dados anteriores.
    NovaGerada,
}

pub fn carregar_ou_criar(servico: &str, config_electron: &Path) -> Result<(MasterKey, OrigemChave)> {
    let entrada = keyring::Entry::new(servico, CONTA).context("cofre de credenciais indisponível")?;
    match entrada.get_password() {
        Ok(hex) => {
            let hex = zeroize::Zeroizing::new(hex);
            return Ok((MasterKey::from_hex(&hex)?, OrigemChave::Cofre));
        }
        Err(keyring::Error::NoEntry) => {}
        Err(e) => return Err(e).context("não foi possível ler a chave no cofre de credenciais"),
    }

    let (chave, origem) = match ler_chave_electron(config_electron)? {
        Some(chave) => (chave, OrigemChave::ImportadaDoElectron),
        None => (MasterKey::generate()?, OrigemChave::NovaGerada),
    };
    entrada
        .set_password(&chave.to_hex())
        .context("não foi possível gravar a chave no cofre de credenciais")?;
    // Confere a gravação antes de seguir: perder a chave torna as conexões ilegíveis.
    let relida = zeroize::Zeroizing::new(entrada.get_password().context("chave gravada não pôde ser relida")?);
    anyhow::ensure!(relida.as_str() == chave.to_hex().as_str(), "a chave relida do cofre não confere");
    Ok((chave, origem))
}

/// Lê `masterKeyHex` do `local-config.json` do Electron, se existir e for válido.
/// O arquivo não é alterado: o app Electron ainda pode estar instalado.
fn ler_chave_electron(caminho: &Path) -> Result<Option<MasterKey>> {
    let texto = match std::fs::read_to_string(caminho) {
        Ok(t) => zeroize::Zeroizing::new(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).context("não foi possível ler o local-config.json do Electron"),
    };
    let json: serde_json::Value = match serde_json::from_str(&texto) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    Ok(json
        .get("masterKeyHex")
        .and_then(|v| v.as_str())
        .and_then(|hex| MasterKey::from_hex(hex).ok()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_chave_do_electron_so_quando_valida() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("local-config.json");
        assert!(ler_chave_electron(&p).unwrap().is_none(), "arquivo ausente");

        std::fs::write(&p, r#"{"masterKeyHex":"nao-hex"}"#).unwrap();
        assert!(ler_chave_electron(&p).unwrap().is_none(), "chave inválida");

        std::fs::write(&p, format!(r#"{{"masterKeyHex":"{}"}}"#, "ab".repeat(32))).unwrap();
        let chave = ler_chave_electron(&p).unwrap().unwrap();
        assert_eq!(chave.to_hex().as_str(), "ab".repeat(32));
    }
}
