//! Banco SQLite: mesmo schema do `server.js`, para que um banco do Electron
//! possa ser importado sem conversão. As migrações podem ser reaplicadas e
//! nunca apagam dados.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
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

/// Índices que custam uma passada pelo ledger inteiro para criar: ficam para a
/// manutenção em segundo plano (`criar_indices_da_manutencao`), depois da
/// compactação, e nunca atrasam a abertura.
///
/// - `INDICE_SEM_CONFIRMACAO`: apostas da Polymarket sem data de resultado
///   confirmada. Sem ele, cada leitura do cache de metadados
///   (`poly::necessidades`) rodava `json_extract` em todas as apostas.
///
/// Só índices parciais: o SQLite só usa um índice parcial numa consulta que
/// repete a condição dele, então nenhuma outra consulta muda de plano. Um
/// índice comum em `closed_bets_ledger(user_id, ...)` fazia o ledger ser lido
/// por ele e mudava a ordem das apostas com o mesmo `first_seen_at`.
const INDICES_DA_MANUTENCAO: &str = r#"
CREATE INDEX IF NOT EXISTS idx_closed_bets_ledger_poly_sem_confirmacao ON closed_bets_ledger(user_id)
  WHERE platform='polymarket' AND json_extract(data_json,'$._confirmedClosedTs') IS NULL;
"#;

/// Nome do índice parcial das apostas da Polymarket sem data confirmada.
pub const INDICE_SEM_CONFIRMACAO: &str = "idx_closed_bets_ledger_poly_sem_confirmacao";

/// O índice existe (a manutenção da abertura já rodou neste banco)?
pub fn tem_indice(conn: &Connection, nome: &str) -> Result<bool> {
    Ok(conn.query_row("SELECT 1 FROM sqlite_master WHERE type='index' AND name=?", [nome], |_| Ok(())).optional()?.is_some())
}

pub fn criar_indices_da_manutencao(conn: &Connection) -> Result<()> {
    conn.execute_batch(INDICES_DA_MANUTENCAO).context("índices da manutenção")
}

pub fn agora() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Leitura mapeada do arquivo do banco, até 256 MB.
const MMAP: i64 = 268_435_456;

/// Conexão só de leitura, com as funções SQL registradas. Em WAL ela lê sem
/// esperar a conexão de escrita e vê tudo o que já foi confirmado.
///
/// Abre sem `mmap_size` (ver `Db::mapear_leitura`).
pub fn abrir_leitura(caminho: &Path) -> Result<Connection> {
    use rusqlite::OpenFlags;
    let conn = Connection::open_with_flags(caminho, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .with_context(|| format!("não foi possível abrir {} para leitura", caminho.display()))?;
    conn.busy_timeout(Duration::from_secs(5))?;
    // Cada conexão tem o próprio cache de páginas: 8 MB, contra 32 MB na de escrita.
    conn.pragma_update(None, "cache_size", -8_000)?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    registrar_funcoes(&conn)?;
    Ok(conn)
}

/// Conexão só de leitura com uma transação aberta. Em WAL ela enxerga o banco
/// como estava ao abrir, mesmo com a sincronização gravando pela conexão
/// principal: é o estado salvo que a interface mostra enquanto atualiza.
pub fn abrir_instantaneo(caminho: &Path) -> Result<Connection> {
    let conn = abrir_leitura(caminho)?;
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
    conn.pragma_update(None, "mmap_size", MMAP)?;
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

/// Uma conexão protegida por mutex; as consultas rodam fora das threads
/// assíncronas para não travar a interface.
#[derive(Clone)]
pub struct Conexao(Arc<Mutex<Connection>>);

impl Conexao {
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

/// Conexões de leitura abertas ao lado da de escrita.
pub const CONEXOES_DE_LEITURA: usize = 2;

/// O banco do app: uma conexão de escrita e algumas só de leitura.
///
/// Em WAL as leituras não esperam as gravações: `ler` vê tudo o que já foi
/// confirmado, mesmo durante a manutenção da abertura ou um lote longo do
/// ledger. Toda gravação passa por `call`, uma por vez.
///
/// A ordem dos campos importa: eles saem na ordem declarada, e a conexão de
/// escrita tem de fechar por último. Só ela consegue juntar o WAL ao arquivo
/// e apagá-lo; se uma de leitura fechar por último, o WAL fica do lado do
/// `betty.db`, e uma cópia só do `.db` sai sem as últimas gravações.
#[derive(Clone)]
pub struct Db {
    leitura: Arc<[Conexao]>,
    vez: Arc<AtomicUsize>,
    escrita: Conexao,
}

impl Db {
    /// Abre `n` conexões de leitura do mesmo arquivo da conexão de escrita.
    /// Com `n = 0`, `ler` usa a de escrita.
    pub fn com_leitura(conn: Connection, n: usize) -> Result<Self> {
        let caminho = conn.path().filter(|p| !p.is_empty()).map(std::path::PathBuf::from).ok_or_else(|| anyhow!("banco sem arquivo"))?;
        let leitura = (0..n).map(|_| abrir_leitura(&caminho).map(Conexao::new)).collect::<Result<Vec<_>>>()?;
        Ok(Self { leitura: leitura.into(), vez: Default::default(), escrita: Conexao::new(conn) })
    }

    /// Liga a leitura mapeada (`mmap_size`) nas conexões de leitura, que deixa
    /// as varreduras grandes do ledger 1,3 a 1,6 vez mais rápidas.
    ///
    /// Só depois da manutenção da abertura: no Windows um arquivo mapeado por
    /// outra conexão não pode ser truncado, e o `VACUUM` de `recuperar_espaco`
    /// deixaria de devolver o espaço ao disco sem avisar.
    pub async fn mapear_leitura(&self) -> Result<()> {
        for conexao in self.leitura.iter() {
            conexao.call(|c| Ok(c.pragma_update(None, "mmap_size", MMAP)?)).await?;
        }
        Ok(())
    }

    /// A conexão de escrita.
    pub async fn call<R, F>(&self, f: F) -> Result<R>
    where
        R: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<R> + Send + 'static,
    {
        self.escrita.call(f).await
    }

    /// Uma conexão só de leitura: a primeira livre, a partir de um rodízio;
    /// com todas ocupadas, espera a da vez. Qualquer gravação dentro de `f`
    /// falha (`SQLITE_READONLY`).
    pub async fn ler<R, F>(&self, f: F) -> Result<R>
    where
        R: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<R> + Send + 'static,
    {
        if self.leitura.is_empty() {
            return self.escrita.call(f).await;
        }
        let leitura = self.leitura.clone();
        let vez = self.vez.fetch_add(1, Ordering::Relaxed);
        tauri::async_runtime::spawn_blocking(move || {
            let n = leitura.len();
            let livre = (0..n).find_map(|i| leitura[(vez + i) % n].0.try_lock().ok());
            let mut guard = match livre {
                Some(g) => g,
                None => leitura[vez % n].0.lock().map_err(|_| anyhow!("banco indisponível"))?,
            };
            f(&mut guard)
        })
        .await
        .map_err(|e| anyhow!("tarefa do banco falhou: {e}"))?
    }
}

#[cfg(test)]
pub(crate) mod tests {
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

    /// Segura a conexão de escrita no meio de uma transação até `soltar`.
    pub(crate) fn segurar_escrita(db: &Db) -> (std::sync::mpsc::Sender<()>, std::thread::JoinHandle<Result<()>>) {
        use std::sync::mpsc;
        let (entrou, dentro) = mpsc::channel();
        let (soltar, solto) = mpsc::channel::<()>();
        let db = db.clone();
        let escrita = std::thread::spawn(move || {
            tauri::async_runtime::block_on(db.call(move |c| {
                let tx = c.transaction()?;
                tx.execute("INSERT INTO database_maintenance(maintenance_key,completed_at) VALUES('teste-escrita','agora')", [])?;
                entrou.send(()).unwrap();
                solto.recv().unwrap();
                tx.commit()?;
                Ok(())
            }))
        });
        dentro.recv().unwrap();
        (soltar, escrita)
    }

    /// Roda `f` numa thread e falha se ela não terminar em 5 s.
    pub(crate) fn sem_esperar<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> R {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        rx.recv_timeout(Duration::from_secs(5)).expect("a leitura ficou esperando a gravação")
    }

    #[test]
    fn leitura_nao_espera_a_gravacao() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = abrir(&dir.path().join("t.db")).unwrap();
        usuario_local(&mut conn).unwrap();
        let db = Db::com_leitura(conn, 2).unwrap();
        let contar = |db: &Db| {
            let db = db.clone();
            sem_esperar(move || {
                tauri::async_runtime::block_on(db.ler(|c| Ok(c.query_row("SELECT COUNT(*) FROM database_maintenance", [], |r| r.get::<_, i64>(0))?)))
                    .unwrap()
            })
        };
        let (soltar, escrita) = segurar_escrita(&db);
        let antes = contar(&db);
        // As duas conexões de leitura atendem ao mesmo tempo.
        let (a, b) = (db.clone(), db.clone());
        let juntas = sem_esperar(move || {
            let (tx, rx) = std::sync::mpsc::channel();
            let (tx2, rx2) = std::sync::mpsc::channel();
            let um = std::thread::spawn(move || {
                tauri::async_runtime::block_on(a.ler(move |_| {
                    tx.send(()).unwrap();
                    rx2.recv().unwrap();
                    Ok(())
                }))
            });
            rx.recv().unwrap();
            let outro = tauri::async_runtime::block_on(b.ler(|c| Ok(c.query_row("SELECT 1", [], |r| r.get::<_, i64>(0))?)));
            tx2.send(()).unwrap();
            um.join().unwrap().unwrap();
            outro.unwrap()
        });
        assert_eq!(juntas, 1);
        soltar.send(()).unwrap();
        escrita.join().unwrap().unwrap();
        assert_eq!(contar(&db), antes + 1, "a leitura vê o que já foi confirmado");

        let erro = tauri::async_runtime::block_on(db.ler(|c| Ok(c.execute("DELETE FROM database_maintenance", [])?))).unwrap_err();
        assert!(format!("{erro:#}").contains("readonly"), "{erro:#}");
    }

    #[test]
    fn vacuum_encolhe_o_arquivo_com_as_leituras_abertas() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.db");
        let conn = abrir(&p).unwrap();
        conn.execute_batch(
            "CREATE TABLE lixo(x BLOB);
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<400) INSERT INTO lixo SELECT randomblob(100000) FROM n;
             DELETE FROM lixo;",
        )
        .unwrap();
        conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(())).unwrap();
        // Com `mapear_leitura` aqui o arquivo continua com 40 MB, sem erro.
        let db = Db::com_leitura(conn, 2).unwrap();
        // As duas conexões de leitura já leram o arquivo.
        let (a, b) = (db.clone(), db.clone());
        sem_esperar(move || {
            let (tx, rx) = std::sync::mpsc::channel();
            let um = std::thread::spawn(move || {
                tauri::async_runtime::block_on(a.ler(move |c| {
                    c.query_row("SELECT COUNT(*) FROM lixo", [], |_| Ok(()))?;
                    rx.recv().unwrap();
                    Ok(())
                }))
            });
            tauri::async_runtime::block_on(b.ler(|c| Ok(c.query_row("SELECT COUNT(*) FROM lixo", [], |_| Ok(()))?))).unwrap();
            tx.send(()).unwrap();
            um.join().unwrap().unwrap();
        });
        let antes = std::fs::metadata(&p).unwrap().len();
        let livre = tauri::async_runtime::block_on(db.call(|c| recuperar_espaco(c))).unwrap();
        assert!(livre.is_some_and(|b| b > LIVRE_PARA_VACUUM), "{livre:?}");
        let depois = std::fs::metadata(&p).unwrap().len();
        assert!(depois < antes / 10, "o arquivo não encolheu: {antes} -> {depois}");
        tauri::async_runtime::block_on(db.mapear_leitura()).unwrap();
        let n = tauri::async_runtime::block_on(db.ler(|c| Ok(c.query_row("SELECT COUNT(*) FROM lixo", [], |r| r.get::<_, i64>(0))?))).unwrap();
        assert_eq!(n, 0, "lê normalmente depois de mapear");
    }

    #[test]
    fn fechar_junta_o_wal_ao_arquivo() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.db");
        let conn = abrir(&p).unwrap();
        // Sem o checkpoint automático, tudo o que foi gravado fica no WAL até fechar.
        conn.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
        let db = Db::com_leitura(conn, 2).unwrap();
        tauri::async_runtime::block_on(db.mapear_leitura()).unwrap();
        tauri::async_runtime::block_on(db.call(|c| Ok(c.execute_batch("CREATE TABLE t(x); INSERT INTO t VALUES(randomblob(100000));")?))).unwrap();
        tauri::async_runtime::block_on(db.ler(|c| Ok(c.query_row("SELECT COUNT(*) FROM t", [], |_| Ok(()))?))).unwrap();
        let wal = dir.path().join("t.db-wal");
        assert!(std::fs::metadata(&wal).unwrap().len() > 100_000, "a gravação está no WAL");
        drop(db);
        assert!(!wal.exists(), "a conexão de escrita fecha por último e apaga o WAL");
        // O arquivo sozinho tem tudo.
        let c = Connection::open(&p).unwrap();
        assert_eq!(c.query_row("SELECT length(x) FROM t", [], |r| r.get::<_, i64>(0)).unwrap(), 100_000);
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
