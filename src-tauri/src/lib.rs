//! Betty em Rust.
//!
//! A interface roda num WebView e conversa com o backend pelo protocolo interno
//! `betty://`, atendido dentro do próprio processo. Diferente da versão Electron,
//! não existe servidor HTTP nem porta TCP aberta: nenhum outro programa da
//! máquina consegue consultar as rotas `/api/*`.

mod api;
mod audit;
mod breakdown;
mod crypto;
mod db;
mod download;
mod history;
mod import;
mod js;
mod kalshi;
mod kalshi_dados;
mod keystore;
mod ledger;
mod net;
mod odds;
mod poly;
mod poly_datas;
mod protocol;
mod titles;
mod tracked;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Context;

#[global_allocator]
static ALOCADOR: mimalloc::MiMalloc = mimalloc::MiMalloc;
use tauri::webview::NewWindowResponse;
use tauri::{App, Manager, WebviewUrl, WebviewWindowBuilder};

pub fn run() {
    tauri::Builder::default()
        // A segunda abertura só traz a janela existente para a frente.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(janela) = app.get_webview_window("main") {
                let _ = janela.unminimize();
                let _ = janela.set_focus();
            }
        }))
        .register_asynchronous_uri_scheme_protocol(protocol::SCHEME, |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            tauri::async_runtime::spawn(async move {
                responder.respond(protocol::handle(&app, request).await);
            });
        })
        .setup(|app| {
            let pastas = Pastas::resolver(app)?;
            let estado = inicializar(&pastas).inspect_err(|e| registrar_falha(&pastas, e))?;
            app.manage(estado);

            let url = protocol::app_url("dashboard.html")?;
            let (app_navegacao, app_janela) = (app.handle().clone(), app.handle().clone());
            WebviewWindowBuilder::new(app, "main", WebviewUrl::CustomProtocol(url))
                .title("Betty")
                .inner_size(1440.0, 920.0)
                .min_inner_size(1080.0, 700.0)
                .maximized(true)
                // Rasterização na CPU: na GPU (D3D11) o WebView2 suaviza SVG com MSAA e as
                // curvas/diagonais do logo e dos gráficos saem serrilhadas (3 níveis de
                // cobertura contra 44 na CPU). A composição continua na GPU. Os três
                // primeiros `--disable-features` são os padrões do wry, que este método
                // substitui (menu "mini", PDF e SmartScreen).
                .additional_browser_args(
                    "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection \
                     --disable-gpu-rasterization",
                )
                // A janela nunca sai do app; links das casas abrem no navegador padrão.
                .on_navigation(move |url| protocol::tratar_navegacao(&app_navegacao, url))
                // `target="_blank"` e `window.open` não criam janelas novas no app.
                .on_new_window(move |url, _| {
                    protocol::abrir_no_navegador(&app_janela, &url);
                    NewWindowResponse::Deny
                })
                // Exportação em CSV: salva na pasta Downloads com o nome sugerido.
                .on_download(download::tratar)
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("erro ao iniciar o Betty");
}

/// Nomes de quando o app se chamava MyBettys. Só existem para trazer os dados de
/// quem já usava o app com o nome antigo; nada novo é gravado com eles.
mod legado {
    /// Pasta de dados da versão Rust anterior, dentro de `%APPDATA%`.
    pub const PASTA_RUST: &str = "com.mybettys.desktop";
    /// Pasta de dados da versão Electron, dentro de `%APPDATA%`.
    pub const PASTA_ELECTRON: &str = "MyBettys";
    pub const BANCO: &str = "mybettys.db";
    pub const SERVICO_COFRE: &str = "MyBettys";
}

struct Pastas {
    /// Dados do app: `%APPDATA%\com.betty.desktop`.
    dados: PathBuf,
    /// Dados da versão Rust com o nome antigo, se houver de onde trazer.
    dados_legado: Option<PathBuf>,
    /// Dados da versão Electron: `%APPDATA%\MyBettys`.
    electron: PathBuf,
    servico_cofre: String,
    servico_cofre_legado: Option<String>,
}

impl Pastas {
    fn resolver(app: &App) -> anyhow::Result<Self> {
        let mut pastas = Self {
            dados: app.path().app_data_dir()?,
            dados_legado: Some(app.path().data_dir()?.join(legado::PASTA_RUST)),
            electron: app.path().data_dir()?.join(legado::PASTA_ELECTRON),
            servico_cofre: "Betty".into(),
            servico_cofre_legado: Some(legado::SERVICO_COFRE.into()),
        };
        // Só em desenvolvimento: permite testar com cópias, sem tocar nos dados reais
        // (por isso quem troca a pasta ou o cofre também deixa de olhar os antigos).
        if cfg!(debug_assertions) {
            if let Ok(v) = std::env::var("BETTY_DATA_DIR") {
                pastas.dados = v.into();
                pastas.dados_legado = std::env::var("BETTY_DATA_DIR_LEGADO").ok().map(Into::into);
            }
            if let Ok(v) = std::env::var("BETTY_ELECTRON_DIR") {
                pastas.electron = v.into();
            }
            if let Ok(v) = std::env::var("BETTY_KEYRING_SERVICE") {
                pastas.servico_cofre = v;
                pastas.servico_cofre_legado = std::env::var("BETTY_KEYRING_SERVICE_LEGADO").ok();
            }
        }
        Ok(pastas)
    }
}

/// Move o banco da versão com o nome antigo para a pasta nova, na primeira vez
/// que o Betty abre. É um `rename` (instantâneo, sem duplicar centenas de MB).
/// O `.db` vai por último: se algo falhar no meio, a próxima abertura repete.
fn trazer_banco_legado(antigo: &std::path::Path, novo: &std::path::Path) -> anyhow::Result<bool> {
    if novo.exists() || !antigo.exists() {
        return Ok(false);
    }
    let com_sufixo = |base: &std::path::Path, sufixo: &str| {
        let mut nome = base.as_os_str().to_owned();
        nome.push(sufixo);
        PathBuf::from(nome)
    };
    for sufixo in ["-wal", "-shm", ""] {
        let de = com_sufixo(antigo, sufixo);
        if de.exists() {
            std::fs::rename(&de, com_sufixo(novo, sufixo))
                .with_context(|| format!("não foi possível trazer {} da pasta antiga", de.display()))?;
        }
    }
    Ok(true)
}

fn inicializar(pastas: &Pastas) -> anyhow::Result<api::Estado> {
    let inicio = Instant::now();
    std::fs::create_dir_all(&pastas.dados).context("não foi possível criar a pasta de dados")?;
    let banco = pastas.dados.join("betty.db");

    if let Some(antiga) = &pastas.dados_legado
        && trazer_banco_legado(&antiga.join(legado::BANCO), &banco)?
    {
        eprintln!("[início] banco trazido da pasta com o nome antigo");
    }
    let importacao = import::importar_se_necessario(&banco, &pastas.electron.join(legado::BANCO))?;
    eprintln!("[início] banco: {importacao:?}");

    let (chave, origem) = keystore::carregar_ou_criar(
        &pastas.servico_cofre,
        pastas.servico_cofre_legado.as_deref(),
        &pastas.electron.join("local-config.json"),
    )?;
    eprintln!("[início] chave mestra: {origem:?}");

    let mut conn = db::abrir(&banco)?;
    let migradas = db::migrar_segredos(&mut conn, &chave)?;
    if migradas > 0 {
        eprintln!("[início] wallets em texto puro cifradas: {migradas}");
    }
    match db::chave_confere(&conn, &chave)? {
        Some(false) => eprintln!("[início] ATENÇÃO: a chave mestra não abre as conexões salvas; será preciso reconectar as casas."),
        Some(true) => eprintln!("[início] chave mestra confere com as conexões salvas"),
        None => {}
    }
    let usuario = db::usuario_local(&mut conn)?;
    eprintln!("[início] pronto em {} ms", inicio.elapsed().as_millis());

    Ok(api::Estado { db: db::Db::new(conn), chave: Arc::new(chave), usuario, kalshi: Default::default(), poly: Default::default(), tracked: Default::default() })
}

fn registrar_falha(pastas: &Pastas, erro: &anyhow::Error) {
    use std::io::Write;
    let texto = format!("{} {erro:#}\n", db::agora());
    eprintln!("[início] falha: {erro:#}");
    let _ = std::fs::create_dir_all(&pastas.dados);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(pastas.dados.join("startup-error.log")) {
        let _ = f.write_all(texto.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traz_o_banco_da_pasta_antiga_uma_unica_vez() {
        let dir = tempfile::tempdir().unwrap();
        let antigo = dir.path().join(legado::BANCO);
        let novo = dir.path().join("nova").join("betty.db");
        std::fs::create_dir_all(novo.parent().unwrap()).unwrap();
        assert!(!trazer_banco_legado(&antigo, &novo).unwrap(), "sem banco antigo não faz nada");

        std::fs::write(&antigo, b"banco").unwrap();
        std::fs::write(dir.path().join("mybettys.db-wal"), b"wal").unwrap();
        assert!(trazer_banco_legado(&antigo, &novo).unwrap());
        assert_eq!(std::fs::read(&novo).unwrap(), b"banco");
        assert_eq!(std::fs::read(novo.with_file_name("betty.db-wal")).unwrap(), b"wal");
        assert!(!antigo.exists(), "o banco foi movido, não copiado");

        std::fs::write(&antigo, b"outro").unwrap();
        assert!(!trazer_banco_legado(&antigo, &novo).unwrap(), "com banco novo, o antigo é ignorado");
        assert_eq!(std::fs::read(&novo).unwrap(), b"banco");
    }
}
