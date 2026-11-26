//! Importação do banco do MyBettys Electron.
//!
//! O banco antigo é aberto somente para leitura e copiado pela API de backup do
//! SQLite, que gera um snapshot consistente mesmo com o Electron aberto. O
//! arquivo original nunca é alterado.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OpenFlags};

#[derive(Debug, PartialEq, Eq)]
pub enum ResultadoImportacao {
    /// O banco novo já existe: nada a importar.
    JaExiste,
    /// Não há banco do Electron nesta máquina.
    SemBancoAnterior,
    Importado { bytes: u64, duracao_ms: u128 },
}

pub fn importar_se_necessario(banco_novo: &Path, banco_electron: &Path) -> Result<ResultadoImportacao> {
    if banco_novo.exists() {
        return Ok(ResultadoImportacao::JaExiste);
    }
    if !banco_electron.exists() {
        return Ok(ResultadoImportacao::SemBancoAnterior);
    }
    let inicio = Instant::now();
    // Grava num arquivo temporário e só renomeia no fim: uma importação
    // interrompida nunca deixa um banco pela metade no lugar do definitivo.
    let temporario = banco_novo.with_extension("db.importando");
    let _ = std::fs::remove_file(&temporario);

    let origem = Connection::open_with_flags(
        banco_electron,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("não foi possível abrir {} para leitura", banco_electron.display()))?;
    origem.busy_timeout(Duration::from_secs(10))?;

    {
        let mut destino = Connection::open(&temporario)?;
        let backup = rusqlite::backup::Backup::new(&origem, &mut destino)?;
        backup
            .run_to_completion(4096, Duration::ZERO, None)
            .context("falha ao copiar o banco do Electron")?;
    }
    drop(origem);

    {
        let destino = Connection::open(&temporario)?;
        let integridade: String = destino.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
        ensure!(integridade == "ok", "a cópia do banco falhou na verificação de integridade: {integridade}");
        destino.execute(
            "CREATE TABLE IF NOT EXISTS database_maintenance (maintenance_key TEXT PRIMARY KEY, completed_at TEXT NOT NULL)",
            [],
        )?;
        destino.execute(
            "INSERT OR REPLACE INTO database_maintenance(maintenance_key,completed_at) VALUES('imported_from_electron_v1',?)",
            [crate::db::agora()],
        )?;
    }

    std::fs::rename(&temporario, banco_novo).context("não foi possível mover o banco importado")?;
    let bytes = std::fs::metadata(banco_novo)?.len();
    Ok(ResultadoImportacao::Importado { bytes, duracao_ms: inicio.elapsed().as_millis() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copia_sem_alterar_o_original() {
        let dir = tempfile::tempdir().unwrap();
        let antigo = dir.path().join("electron.db");
        let novo = dir.path().join("novo").join("mybettys.db");
        std::fs::create_dir_all(novo.parent().unwrap()).unwrap();

        assert_eq!(importar_se_necessario(&novo, &antigo).unwrap(), ResultadoImportacao::SemBancoAnterior);

        {
            let c = Connection::open(&antigo).unwrap();
            c.pragma_update(None, "journal_mode", "WAL").unwrap();
            c.execute_batch(
                "CREATE TABLE closed_bets_ledger (user_id TEXT, platform TEXT, bet_key TEXT, data_json TEXT);
                 INSERT INTO closed_bets_ledger VALUES ('u','kalshi','k1','{\"a\":1}'), ('u','polymarket','p1','{}');",
            )
            .unwrap();
        }
        let antes = std::fs::read(&antigo).unwrap();

        let r = importar_se_necessario(&novo, &antigo).unwrap();
        assert!(matches!(r, ResultadoImportacao::Importado { .. }));
        assert_eq!(std::fs::read(&antigo).unwrap(), antes, "o banco do Electron não pode mudar");

        let c = Connection::open(&novo).unwrap();
        let n: i64 = c.query_row("SELECT COUNT(*) FROM closed_bets_ledger", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 2);
        assert!(!novo.with_extension("db.importando").exists());

        assert_eq!(importar_se_necessario(&novo, &antigo).unwrap(), ResultadoImportacao::JaExiste);
    }
}
