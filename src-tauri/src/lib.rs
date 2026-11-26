//! MyBettys em Rust.
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
            WebviewWindowBuilder::new(app, "main", WebviewUrl::CustomProtocol(url))
                .title("MyBettys")
                .inner_size(1440.0, 920.0)
                .min_inner_size(1080.0, 700.0)
                .maximized(true)
                .on_navigation(protocol::navegacao_permitida)
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("erro ao iniciar o MyBettys");
}

struct Pastas {
    /// Dados do app Rust: `%APPDATA%\com.mybettys.desktop`.
    dados: PathBuf,
    /// Dados do MyBettys Electron: `%APPDATA%\MyBettys`.
    electron: PathBuf,
    servico_cofre: String,
}

impl Pastas {
    fn resolver(app: &App) -> anyhow::Result<Self> {
        let mut pastas = Self {
            dados: app.path().app_data_dir()?,
            electron: app.path().data_dir()?.join("MyBettys"),
            servico_cofre: "MyBettys".into(),
        };
        // Só em desenvolvimento: permite testar com cópias, sem tocar nos dados reais.
        if cfg!(debug_assertions) {
            if let Ok(v) = std::env::var("MYBETTYS_DATA_DIR") {
                pastas.dados = v.into();
            }
            if let Ok(v) = std::env::var("MYBETTYS_ELECTRON_DIR") {
                pastas.electron = v.into();
            }
            if let Ok(v) = std::env::var("MYBETTYS_KEYRING_SERVICE") {
                pastas.servico_cofre = v;
            }
        }
        Ok(pastas)
    }
}

fn inicializar(pastas: &Pastas) -> anyhow::Result<api::Estado> {
    let inicio = Instant::now();
    std::fs::create_dir_all(&pastas.dados).context("não foi possível criar a pasta de dados")?;
    let banco = pastas.dados.join("mybettys.db");

    let importacao = import::importar_se_necessario(&banco, &pastas.electron.join("mybettys.db"))?;
    eprintln!("[início] banco: {importacao:?}");

    let (chave, origem) = keystore::carregar_ou_criar(&pastas.servico_cofre, &pastas.electron.join("local-config.json"))?;
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
