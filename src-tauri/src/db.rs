//! Banco SQLite: mesmo schema do `server.js`, para que um banco do Electron
//! possa ser importado sem conversão. As migrações podem ser reaplicadas e
//! nunca apagam dados.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::crypto::MasterKey;

/// Tabelas, na forma criada pelo `server.js` e por `lib/polymarket-activity-cache.js`.
/// `users` e `sessions` só existem por compatibilidade: o app desktop não tem login.
const TABELAS: &str = r#"
CREATE TABLE IF NOT EXISTS users (
  id TEXT PRIMARY KEY,
  email TEXT NOT NULL UNIQUE,
  password_hash TEXT NOT NULL,
  display_name TEXT NOT NULL,
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS sessions (
  token_hash TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  expires_at TEXT NOT NULL,
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS connections (
  user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  polymarket_wallet TEXT,
  polymarket_wallet_enc TEXT,
  kalshi_api_key_enc TEXT,
  kalshi_private_key_enc TEXT,
  updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS tracked_connections (
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  platform TEXT NOT NULL,
  account_scope TEXT NOT NULL,
  credentials_enc TEXT,
  PRIMARY KEY (user_id, platform)
);
CREATE TABLE IF NOT EXISTS settings (
  user_id TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  stake_unidade REAL NOT NULL DEFAULT 1,
  stake_regras TEXT NOT NULL DEFAULT '[]',
  closed_range_start TEXT NOT NULL DEFAULT '',
  closed_range_end TEXT NOT NULL DEFAULT '',
  closed_range_by_platform TEXT NOT NULL DEFAULT '',
  updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS closed_bets_ledger (
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  platform TEXT NOT NULL,
  bet_key TEXT NOT NULL,
  data_json TEXT NOT NULL,
  first_seen_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  PRIMARY KEY (user_id, platform, bet_key)
);
CREATE TABLE IF NOT EXISTS closed_bets_index (
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  platform TEXT NOT NULL,
  bet_key TEXT NOT NULL,
  category TEXT NOT NULL,
  start_ymd TEXT NOT NULL DEFAULT '',
  table_ymd TEXT NOT NULL DEFAULT '',
  table_ts INTEGER NOT NULL DEFAULT 0,
  pnl_ts INTEGER NOT NULL DEFAULT 0,
  pnl_ymd TEXT NOT NULL DEFAULT '',
  pnl_value REAL NOT NULL DEFAULT 0,
  stake_value REAL NOT NULL DEFAULT 0,
  odd_value REAL NOT NULL DEFAULT 0,
  won INTEGER NOT NULL DEFAULT 0,
  source_signature TEXT NOT NULL DEFAULT '',
  metrics_ready INTEGER NOT NULL DEFAULT 0,
  title_search TEXT NOT NULL DEFAULT '',
  title_display TEXT NOT NULL DEFAULT '',
  updated_at TEXT NOT NULL,
  PRIMARY KEY (user_id, platform, bet_key),
  FOREIGN KEY (user_id, platform, bet_key) REFERENCES closed_bets_ledger(user_id, platform, bet_key) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS kalshi_settlement_cache (
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  settlement_key TEXT NOT NULL,
  source_signature TEXT NOT NULL,
  data_json TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  PRIMARY KEY (user_id, settlement_key)
);
CREATE TABLE IF NOT EXISTS kalshi_metadata_cache (
  kind TEXT NOT NULL,
  cache_key TEXT NOT NULL,
  data_json TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  PRIMARY KEY (kind, cache_key)
);
CREATE TABLE IF NOT EXISTS database_maintenance (
  maintenance_key TEXT PRIMARY KEY,
  completed_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS polymarket_metadata_cache (
  user_id INTEGER NOT NULL,
  slug TEXT NOT NULL,
  version INTEGER NOT NULL,
  data_json TEXT NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (user_id,slug)
);
CREATE TABLE IF NOT EXISTS polymarket_activity_cache (
  user_id TEXT NOT NULL, cache_key TEXT NOT NULL, data_json TEXT NOT NULL,
  updated_at INTEGER NOT NULL, PRIMARY KEY(user_id,cache_key)
);
CREATE TABLE IF NOT EXISTS history_index_versions (
  user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  kind TEXT NOT NULL,
  version TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  PRIMARY KEY (user_id, kind)
);
"#;

/// Regras derivadas do índice do histórico cuja versão a interface confere
/// (categoria, título e data). No Electron ficavam no localStorage: um perfil
/// novo do WebView fazia o índice inteiro ser refeito.
pub const VERSOES_INDICE: [&str; 3] = ["category", "title", "date"];

/// Colunas acrescentadas ao longo das versões do Electron. Bancos antigos
/// recebem cada uma só se ainda não existir.
const COLUNAS: &[(&str, &str, &str)] = &[
    ("connections", "polymarket_wallet_enc", "TEXT"),
    ("settings", "stake_por_casa", "TEXT NOT NULL DEFAULT ''"),
    ("settings", "closed_range_start", "TEXT NOT NULL DEFAULT ''"),
    ("settings", "closed_range_end", "TEXT NOT NULL DEFAULT ''"),
    ("settings", "closed_range_by_platform", "TEXT NOT NULL DEFAULT ''"),
    ("closed_bets_index", "pnl_ts", "INTEGER NOT NULL DEFAULT 0"),
    ("closed_bets_index", "title_version", "TEXT NOT NULL DEFAULT ''"),
    ("closed_bets_index", "pnl_ymd", "TEXT NOT NULL DEFAULT ''"),
    ("closed_bets_index", "pnl_value", "REAL NOT NULL DEFAULT 0"),
    ("closed_bets_index", "stake_value", "REAL NOT NULL DEFAULT 0"),
    ("closed_bets_index", "odd_value", "REAL NOT NULL DEFAULT 0"),
    ("closed_bets_index", "won", "INTEGER NOT NULL DEFAULT 0"),
    ("closed_bets_index", "source_signature", "TEXT NOT NULL DEFAULT ''"),
    ("closed_bets_index", "metrics_ready", "INTEGER NOT NULL DEFAULT 0"),
];

/// Índices ficam depois das colunas: alguns dependem de colunas acrescentadas.
const INDICES: &str = r#"
CREATE INDEX IF NOT EXISTS idx_closed_bets_ledger_user ON closed_bets_ledger(user_id);
CREATE INDEX IF NOT EXISTS idx_closed_bets_index_period ON closed_bets_index(user_id, start_ymd);
CREATE INDEX IF NOT EXISTS idx_closed_bets_index_table_date ON closed_bets_index(user_id, table_ymd);
CREATE INDEX IF NOT EXISTS idx_closed_bets_index_category ON closed_bets_index(user_id, category);
CREATE INDEX IF NOT EXISTS idx_closed_bets_index_pnl_date ON closed_bets_index(user_id, pnl_ts);
CREATE INDEX IF NOT EXISTS idx_closed_bets_index_page ON closed_bets_index(user_id, table_ts DESC);
CREATE INDEX IF NOT EXISTS idx_closed_bets_index_platform_table_date ON closed_bets_index(user_id, platform, table_ymd);
CREATE INDEX IF NOT EXISTS idx_closed_bets_index_platform_pnl_date ON closed_bets_index(user_id, platform, pnl_ts);
"#;

pub fn agora() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Conexão só de leitura com uma transação aberta. Em WAL ela enxerga o banco
/// como estava ao abrir, mesmo com a sincronização gravando pela conexão
/// principal: é o estado salvo que a interface mostra enquanto atualiza.
pub fn abrir_instantaneo(caminho: &Path) -> Result<Connection> {
    use rusqlite::OpenFlags;
    let conn = Connection::open_with_flags(caminho, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .with_context(|| format!("não foi possível abrir {} para leitura", caminho.display()))?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.pragma_update(None, "cache_size", -8_000)?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    registrar_funcoes(&conn)?;
    conn.execute_batch("BEGIN")?;
    // O instantâneo só fica fixo na primeira leitura da transação.
    conn.query_row("SELECT COUNT(*) FROM closed_bets_index", [], |_| Ok(()))?;
    Ok(conn)
}

pub fn abrir(caminho: &Path) -> Result<Connection> {
    let conn = Connection::open(caminho).with_context(|| format!("não foi possível abrir {}", caminho.display()))?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    // O padrão do SQLite (2 MB de cache, temporários em disco) faz ordenações
    // grandes irem para arquivos temporários. 32 MB de cache, temporários em
    // memória e leitura mapeada (até 256 MB) evitam isso.
    conn.pragma_update(None, "cache_size", -32_000)?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    conn.pragma_update(None, "mmap_size", 268_435_456i64)?;
    registrar_funcoes(&conn)?;
    migrar(&conn)?;
    Ok(conn)
}

/// Funções usadas nas consultas de estatísticas. No Electron eram funções
/// JavaScript registradas no better-sqlite3.
pub fn registrar_funcoes(conn: &Connection) -> Result<()> {
    use rusqlite::functions::FunctionFlags;
    use rusqlite::types::ValueRef;
    let flags = FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC;
    // O JS recebia número (ou string/null); `Number.isFinite` não converte texto.
    fn numero(v: ValueRef<'_>) -> f64 {
        match v {
            ValueRef::Integer(i) => i as f64,
            ValueRef::Real(f) => f,
            _ => f64::NAN,
        }
    }
    fn texto(v: ValueRef<'_>) -> String {
        match v {
            ValueRef::Text(t) => String::from_utf8_lossy(t).into_owned(),
            ValueRef::Integer(i) => i.to_string(),
            ValueRef::Real(f) => f.to_string(),
            _ => String::new(),
        }
    }
    conn.create_scalar_function("bet_round_odd", 1, flags, |ctx| Ok(crate::odds::arredondar(numero(ctx.get_raw(0)))))?;
    conn.create_scalar_function("bet_market_group", 2, flags, |ctx| {
        Ok(crate::breakdown::chave(&texto(ctx.get_raw(0)), &texto(ctx.get_raw(1))))
    })?;
    conn.create_scalar_function("bet_summary_group", 3, flags, |ctx| {
        Ok(crate::breakdown::chave_resumo(&texto(ctx.get_raw(0)), &texto(ctx.get_raw(1)), numero(ctx.get_raw(2))))
    })?;
    Ok(())
}

pub fn migrar(conn: &Connection) -> Result<()> {
    conn.execute_batch(TABELAS).context("criação das tabelas")?;
    for (tabela, coluna, definicao) in COLUNAS {
        if !tem_coluna(conn, tabela, coluna)? {
            conn.execute_batch(&format!("ALTER TABLE {tabela} ADD COLUMN {coluna} {definicao}"))
                .with_context(|| format!("migração {tabela}.{coluna}"))?;
        }
    }
    conn.execute_batch(INDICES).context("criação dos índices")?;
    analisar_indices_uma_vez(conn)?;
    Ok(())
}

fn tem_coluna(conn: &Connection, tabela: &str, coluna: &str) -> Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({tabela})"))?;
    let nomes = stmt.query_map([], |r| r.get::<_, String>(1))?;
    for nome in nomes {
        if nome? == coluna {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Mesmo marcador do Electron: o ANALYZE roda uma única vez, quando já há histórico.
fn analisar_indices_uma_vez(conn: &Connection) -> Result<()> {
    const CHAVE: &str = "analyze_closed_bets_platform_indexes_v1";
    let feito: bool = conn
        .query_row("SELECT 1 FROM database_maintenance WHERE maintenance_key=?", [CHAVE], |_| Ok(true))
        .optional()?
        .unwrap_or(false);
    if feito {
        return Ok(());
    }
    let total: i64 = conn.query_row("SELECT COUNT(*) FROM closed_bets_index", [], |r| r.get(0))?;
    if total == 0 {
        return Ok(());
    }
    conn.execute_batch("ANALYZE closed_bets_index;")?;
    conn.execute(
        "INSERT INTO database_maintenance(maintenance_key,completed_at) VALUES(?,?)",
        params![CHAVE, agora()],
    )?;
    Ok(())
}

/// Versões das regras com que o índice do histórico foi montado.
pub fn versoes_indice(conn: &Connection, user_id: &str) -> Result<serde_json::Map<String, serde_json::Value>> {
    let mut s = conn.prepare("SELECT kind, version FROM history_index_versions WHERE user_id=? ORDER BY kind")?;
    let linhas = s.query_map([user_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut mapa = serde_json::Map::new();
    for linha in linhas {
        let (tipo, versao) = linha?;
        mapa.insert(tipo, serde_json::Value::String(versao));
    }
    Ok(mapa)
}

/// Grava as versões recebidas; tipos desconhecidos e valores vazios são ignorados.
pub fn salvar_versoes_indice(conn: &mut Connection, user_id: &str, versoes: Option<&serde_json::Value>) -> Result<usize> {
    let Some(mapa) = versoes.and_then(serde_json::Value::as_object) else { return Ok(0) };
    let tx = conn.transaction()?;
    let mut gravadas = 0;
    for tipo in VERSOES_INDICE {
        let Some(versao) = mapa.get(tipo).and_then(serde_json::Value::as_str).map(str::trim).filter(|v| !v.is_empty()) else {
            continue;
        };
        tx.execute(
            "INSERT INTO history_index_versions(user_id,kind,version,updated_at) VALUES(?,?,?,?)
             ON CONFLICT(user_id,kind) DO UPDATE SET version=excluded.version,updated_at=excluded.updated_at",
            params![user_id, tipo, crate::js::fatia_utf16(versao, 40), agora()],
        )?;
        gravadas += 1;
    }
    tx.commit()?;
    Ok(gravadas)
}

/// Espaço livre a partir do qual vale reescrever o arquivo do banco.
const LIVRE_PARA_VACUUM: i64 = 32 * 1024 * 1024;

/// Devolve ao disco o espaço livre do arquivo (`VACUUM`) quando ele passa de
/// 32 MB, por exemplo depois de compactar o ledger, e encolhe o WAL. Devolve
/// quantos bytes estavam livres, ou `None` quando não foi preciso.
pub fn recuperar_espaco(conn: &Connection) -> Result<Option<i64>> {
    let livres: i64 = conn.query_row("PRAGMA freelist_count", [], |r| r.get(0))?;
    let pagina: i64 = conn.query_row("PRAGMA page_size", [], |r| r.get(0))?;
    let bytes = livres * pagina;
    if bytes < LIVRE_PARA_VACUUM {
        return Ok(None);
    }
    conn.execute_batch("VACUUM;")?;
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
    Ok(Some(bytes))
}

/// Ajustes de segurança herdados do Electron:
/// - wallets Polymarket antigas em texto puro passam a ser cifradas;
/// - a coluna legada de Private Key da Polymarket é esvaziada, se existir.
pub fn migrar_segredos(conn: &mut Connection, chave: &MasterKey) -> Result<usize> {
    let tx = conn.transaction()?;
    let legado: Vec<(String, String)> = {
        let mut stmt = tx.prepare(
            "SELECT user_id, polymarket_wallet FROM connections WHERE polymarket_wallet IS NOT NULL \
             AND polymarket_wallet <> '' AND (polymarket_wallet_enc IS NULL OR polymarket_wallet_enc = '')",
        )?;
        let linhas = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        linhas.collect::<rusqlite::Result<_>>()?
    };
    for (user_id, wallet) in &legado {
        tx.execute(
            "UPDATE connections SET polymarket_wallet_enc=?, polymarket_wallet=NULL, updated_at=? WHERE user_id=?",
            params![chave.encrypt(wallet)?, agora(), user_id],
        )?;
    }
    if tem_coluna(&tx, "connections", "polymarket_private_key_enc")? {
        tx.execute(
            "UPDATE connections SET polymarket_private_key_enc=NULL WHERE polymarket_private_key_enc IS NOT NULL",
            [],
        )?;
    }
    tx.commit()?;
    Ok(legado.len())
}

/// Confere se a chave abre os dados cifrados já existentes. `None` quando
/// ainda não há nenhum dado cifrado.
pub fn chave_confere(conn: &Connection, chave: &MasterKey) -> Result<Option<bool>> {
    let amostra: Option<String> = conn
        .query_row(
            "SELECT v FROM (
               SELECT polymarket_wallet_enc AS v FROM connections
               UNION ALL SELECT kalshi_api_key_enc FROM connections
               UNION ALL SELECT credentials_enc FROM tracked_connections
             ) WHERE v IS NOT NULL AND v <> '' LIMIT 1",
            [],
            |r| r.get(0),
        )
        .optional()?;
    Ok(amostra.map(|v| chave.decrypt(&v).is_ok()))
}

#[derive(Debug, Clone, Serialize)]
pub struct UsuarioLocal {
    pub id: String,
    pub email: String,
    pub name: String,
}

/// Equivalente ao `getLocalUser()` do modo Electron: o primeiro usuário do
/// banco, criado na primeira execução.
pub fn usuario_local(conn: &mut Connection) -> Result<UsuarioLocal> {
    let existente = conn
        .query_row(
            "SELECT id, email, display_name FROM users ORDER BY created_at ASC LIMIT 1",
            [],
            |r| Ok(UsuarioLocal { id: r.get(0)?, email: r.get(1)?, name: r.get(2)? }),
        )
        .optional()?;
    let t = agora();
    let tx = conn.transaction()?;
    let usuario = match existente {
        Some(u) => u,
        None => {
            let u = UsuarioLocal {
                id: uuid::Uuid::new_v4().to_string(),
                email: "local@betty.local".into(),
                name: "Usuário Local".into(),
            };
            // Não existe senha no app desktop: "!" nunca corresponde a um hash válido.
            tx.execute(
                "INSERT INTO users(id,email,password_hash,display_name,created_at) VALUES(?,?,?,?,?)",
                params![u.id, u.email, "!", u.name, t],
            )?;
            u
        }
    };
    tx.execute("INSERT OR IGNORE INTO connections(user_id,updated_at) VALUES(?,?)", params![usuario.id, t])?;
    tx.execute("INSERT OR IGNORE INTO settings(user_id,updated_at) VALUES(?,?)", params![usuario.id, t])?;
    tx.commit()?;
    Ok(usuario)
}

/// Conexão única protegida por mutex; as consultas rodam fora das threads
/// assíncronas para não travar a interface.
#[derive(Clone)]
pub struct Db(Arc<Mutex<Connection>>);

impl Db {
    pub fn new(conn: Connection) -> Self {
        Self(Arc::new(Mutex::new(conn)))
    }

    pub async fn call<R, F>(&self, f: F) -> Result<R>
    where
        R: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<R> + Send + 'static,
    {
        let conn = self.0.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let mut guard = conn.lock().map_err(|_| anyhow!("banco indisponível"))?;
            f(&mut guard)
        })
        .await
        .map_err(|e| anyhow!("tarefa do banco falhou: {e}"))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chave() -> MasterKey {
        MasterKey::from_hex(&"11".repeat(32)).unwrap()
    }

    #[test]
    fn migracao_pode_ser_reaplicada() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.db");
        drop(abrir(&p).unwrap());
        let conn = abrir(&p).unwrap();
        assert!(tem_coluna(&conn, "closed_bets_index", "title_version").unwrap());
        assert!(tem_coluna(&conn, "settings", "stake_por_casa").unwrap());
    }

    #[test]
    fn migra_banco_antigo_sem_colunas_novas() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("antigo.db");
        {
            // Formato da primeira versão do Betty (v52build).
            let c = Connection::open(&p).unwrap();
            c.execute_batch(
                "CREATE TABLE users (id TEXT PRIMARY KEY, email TEXT NOT NULL UNIQUE, password_hash TEXT NOT NULL, display_name TEXT NOT NULL, created_at TEXT NOT NULL);
                 CREATE TABLE connections (user_id TEXT PRIMARY KEY, polymarket_wallet TEXT, kalshi_api_key_enc TEXT, kalshi_private_key_enc TEXT, polymarket_private_key_enc TEXT, updated_at TEXT NOT NULL);
                 CREATE TABLE settings (user_id TEXT PRIMARY KEY, poly REAL NOT NULL DEFAULT 0, kalshi REAL NOT NULL DEFAULT 0, stake_unidade REAL NOT NULL DEFAULT 1, stake_regras TEXT NOT NULL DEFAULT '[]', updated_at TEXT NOT NULL);
                 CREATE TABLE closed_bets_index (user_id TEXT NOT NULL, platform TEXT NOT NULL, bet_key TEXT NOT NULL, category TEXT NOT NULL, start_ymd TEXT NOT NULL DEFAULT '', table_ymd TEXT NOT NULL DEFAULT '', table_ts INTEGER NOT NULL DEFAULT 0, title_search TEXT NOT NULL DEFAULT '', title_display TEXT NOT NULL DEFAULT '', updated_at TEXT NOT NULL, PRIMARY KEY (user_id, platform, bet_key));
                 INSERT INTO users VALUES ('u1','local@betty.local','x','Usuário Local','2026-08-26T00:00:00.000Z');
                 INSERT INTO connections(user_id,polymarket_wallet,polymarket_private_key_enc,updated_at) VALUES ('u1','0x1111111111111111111111111111111111111111','segredo-antigo','2026-08-26T00:00:00.000Z');",
            )
            .unwrap();
        }
        let mut conn = abrir(&p).unwrap();
        assert!(tem_coluna(&conn, "closed_bets_index", "pnl_ts").unwrap());
        assert_eq!(migrar_segredos(&mut conn, &chave()).unwrap(), 1);

        let (plano, cifrado, pk): (Option<String>, String, Option<String>) = conn
            .query_row(
                "SELECT polymarket_wallet, polymarket_wallet_enc, polymarket_private_key_enc FROM connections",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(plano, None, "wallet em texto puro é apagada");
        assert_eq!(pk, None, "Private Key legada é apagada");
        assert_eq!(chave().decrypt(&cifrado).unwrap().as_str(), "0x1111111111111111111111111111111111111111");
        assert_eq!(chave_confere(&conn, &chave()).unwrap(), Some(true));
        assert_eq!(
            chave_confere(&conn, &MasterKey::from_hex(&"22".repeat(32)).unwrap()).unwrap(),
            Some(false)
        );

        let u = usuario_local(&mut conn).unwrap();
        assert_eq!(u.id, "u1", "reaproveita o usuário existente");
    }

    #[test]
    fn cria_usuario_local_uma_vez() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = abrir(&dir.path().join("t.db")).unwrap();
        assert_eq!(chave_confere(&conn, &chave()).unwrap(), None);
        let a = usuario_local(&mut conn).unwrap();
        let b = usuario_local(&mut conn).unwrap();
        assert_eq!(a.id, b.id);
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM settings", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
    }
}
