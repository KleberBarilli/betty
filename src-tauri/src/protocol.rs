//! Protocolo interno `betty://`: entrega os arquivos da interface e encaminha
//! `/api/*` e `/health` para o backend.

use tauri::http::{HeaderValue, Method, Request, Response, StatusCode, header};
use tauri::{AppHandle, Manager, Runtime, Url};

use crate::api;

pub const SCHEME: &str = "betty";

/// No Windows o WebView2 expõe protocolos próprios como `http://<nome>.localhost`.
#[cfg(windows)]
const ORIGEM: &str = "http://betty.localhost";
#[cfg(not(windows))]
const ORIGEM: &str = "betty://localhost";

/// Scripts só dos arquivos do próprio app: nenhum script ou handler inline
/// roda (as ações do HTML passam por `ui/acoes.js`). Estilos inline ainda são
/// permitidos. Todo o resto fica restrito à própria origem: nenhuma consulta
/// externa sai do navegador.
const CSP: &str = "default-src 'self'; script-src 'self'; \
    style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; \
    connect-src 'self'; object-src 'none'; base-uri 'none'; form-action 'none'; \
    frame-ancestors 'none'";

pub fn app_url(pagina: &str) -> Result<Url, tauri::Error> {
    Url::parse(&format!("{ORIGEM}/{pagina}")).map_err(|e| tauri::Error::Anyhow(e.into()))
}

/// Só permite navegar dentro do próprio app.
pub fn navegacao_permitida(url: &Url) -> bool {
    url.as_str().starts_with(&format!("{ORIGEM}/"))
}

/// Sites das casas que podem ser abertos no navegador do sistema (o domínio
/// e os subdomínios). Qualquer outro endereço é ignorado.
const DOMINIOS_EXTERNOS: [&str; 4] = ["polymarket.com", "kalshi.com", "predict.fun", "sx.bet"];

/// Link externo aceito: só `https`, sem usuário/senha nem porta própria, com
/// host de nome (não IP) igual a um dos domínios da lista ou subdomínio dele.
/// O `Url` já chega com o host normalizado (minúsculas e punycode).
pub fn link_externo_permitido(url: &Url) -> bool {
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() || url.port().is_some() {
        return false;
    }
    let Some(url::Host::Domain(host)) = url.host() else { return false };
    DOMINIOS_EXTERNOS.iter().any(|d| {
        host == *d
            || host
                .strip_suffix(d)
                .and_then(|resto| resto.strip_suffix('.'))
                .is_some_and(|sub| !sub.is_empty() && sub.split('.').all(|rotulo| !rotulo.is_empty()))
    })
}

/// Pedido de navegação para fora do app (link comum ou `target="_blank"`).
/// A janela nunca sai do app: endereços da lista abrem no navegador padrão e
/// o resto é descartado. Devolve `true` só para navegação interna.
pub fn tratar_navegacao<R: Runtime>(app: &AppHandle<R>, url: &Url) -> bool {
    if navegacao_permitida(url) {
        return true;
    }
    abrir_no_navegador(app, url);
    false
}

/// Abre um link da lista no navegador padrão do sistema.
pub fn abrir_no_navegador<R: Runtime>(app: &AppHandle<R>, url: &Url) {
    if !link_externo_permitido(url) {
        eprintln!("[links] endereço externo bloqueado: {}", url.scheme());
        return;
    }
    let endereco = url.as_str().to_owned();
    // Só em desenvolvimento: registra o link sem abrir o navegador.
    if cfg!(debug_assertions) && std::env::var_os("BETTY_LINKS_SEM_ABRIR").is_some() {
        eprintln!("[links] abriria no navegador: {endereco}");
        return;
    }
    // Fora do callback do WebView2 e na thread principal (COM já iniciado),
    // como o ShellExecuteEx espera.
    let _ = app.run_on_main_thread(move || {
        if let Err(e) = tauri_plugin_opener::open_url(&endereco, None::<&str>) {
            eprintln!("[links] não foi possível abrir o navegador: {e}");
        }
    });
}

pub async fn handle<R: Runtime>(app: &AppHandle<R>, request: Request<Vec<u8>>) -> Response<Vec<u8>> {
    let path = request.uri().path().to_owned();
    if path == "/api/atualizacao" || path == "/api/atualizacao/instalar" {
        return atualizacao(app, request.method(), &path, request.uri().query()).await;
    }
    if path == "/health" || path.starts_with("/api/") {
        let inicio = std::time::Instant::now();
        let estado = app.state::<api::Estado>();
        let resposta = api::handle(&estado, request.method(), &path, request.uri().query(), request.body()).await;
        let corpo = match resposta.bruto {
            Some(bytes) => bytes,
            None => crate::js::para_vec(&resposta.body),
        };
        // Diagnóstico de desempenho, só em desenvolvimento: BETTY_LOG_API=1.
        if cfg!(debug_assertions) && std::env::var_os("BETTY_LOG_API").is_some() {
            eprintln!(
                "[api-tempo] {} {path} {} recebido={} enviado={} {:.1}ms",
                request.method(),
                resposta.status.as_u16(),
                request.body().len(),
                corpo.len(),
                inicio.elapsed().as_secs_f64() * 1000.0
            );
        }
        return montar(resposta.status, "application/json; charset=utf-8", corpo, true);
    }
    if request.method() != Method::GET && request.method() != Method::HEAD {
        return montar(StatusCode::METHOD_NOT_ALLOWED, "text/plain; charset=utf-8", Vec::new(), true);
    }
    arquivo_estatico(app, &path)
}

/// Rotas da atualização: ficam aqui porque precisam da versão do app e de
/// fechá-lo depois de abrir o instalador.
async fn atualizacao<R: Runtime>(app: &AppHandle<R>, metodo: &Method, path: &str, query: Option<&str>) -> Response<Vec<u8>> {
    let versao = app.package_info().version.to_string();
    let json = |status: StatusCode, v: serde_json::Value| montar(status, "application/json; charset=utf-8", crate::js::para_vec(&v), true);
    match (metodo.as_str(), path) {
        ("GET", "/api/atualizacao") => {
            let forcar = query.is_some_and(|q| q.split('&').any(|p| p == "forcar=1"));
            json(StatusCode::OK, crate::atualizacao::consultar(&versao, forcar).await)
        }
        ("POST", "/api/atualizacao/instalar") => {
            // Uma instalação por vez, mesmo com cliques repetidos.
            static EM_ANDAMENTO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if EM_ANDAMENTO.swap(true, std::sync::atomic::Ordering::SeqCst) {
                return json(StatusCode::CONFLICT, serde_json::json!({ "error": "A atualização já está em andamento." }));
            }
            let resultado = match crate::atualizacao::baixar(&versao).await {
                Ok(caminho) => crate::atualizacao::executar(&caminho),
                Err(e) => Err(e),
            };
            match resultado {
                Ok(()) => {
                    // Fecha o Betty logo depois de a resposta chegar à página; o
                    // instalador espera ele sair (e o fecha, se preciso).
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
                        app.exit(0);
                    });
                    json(StatusCode::OK, serde_json::json!({ "ok": true }))
                }
                Err(e) => {
                    EM_ANDAMENTO.store(false, std::sync::atomic::Ordering::SeqCst);
                    eprintln!("[atualização] falhou: {e:#}");
                    json(StatusCode::BAD_GATEWAY, serde_json::json!({ "error": format!("Não foi possível atualizar: {e}.") }))
                }
            }
        }
        _ => montar(StatusCode::METHOD_NOT_ALLOWED, "text/plain; charset=utf-8", Vec::new(), true),
    }
}

fn arquivo_estatico<R: Runtime>(app: &AppHandle<R>, path: &str) -> Response<Vec<u8>> {
    let relativo = match path.trim_start_matches('/') {
        "" | "index.html" => "dashboard.html",
        outro => outro,
    };
    let resolver = app.asset_resolver();
    // Igual ao backend antigo: rotas sem extensão abrem o dashboard.
    let asset = resolver.get(relativo.to_string()).or_else(|| {
        if relativo.contains('.') { None } else { resolver.get("dashboard.html".to_string()) }
    });
    match asset {
        Some(asset) => montar(StatusCode::OK, &asset.mime_type, asset.bytes, false),
        None => montar(StatusCode::NOT_FOUND, "text/plain; charset=utf-8", b"Not found".to_vec(), true),
    }
}

fn montar(status: StatusCode, tipo: &str, corpo: Vec<u8>, sem_cache: bool) -> Response<Vec<u8>> {
    let mut resposta = Response::new(corpo);
    *resposta.status_mut() = status;
    let headers = resposta.headers_mut();
    if let Ok(valor) = HeaderValue::from_str(tipo) {
        headers.insert(header::CONTENT_TYPE, valor);
    }
    headers.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(CSP));
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    headers.insert("Cross-Origin-Opener-Policy", HeaderValue::from_static("same-origin"));
    headers.insert(
        "Permissions-Policy",
        HeaderValue::from_static("camera=(), microphone=(), geolocation=(), payment=(), usb=()"),
    );
    if sem_cache {
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    resposta
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navegacao_fica_dentro_do_app() {
        assert!(navegacao_permitida(&app_url("dashboard.html").unwrap()));
        assert!(!navegacao_permitida(&Url::parse("https://polymarket.com/").unwrap()));
        assert!(!navegacao_permitida(&Url::parse("http://betty.localhost.evil.com/").unwrap()));
    }

    #[test]
    fn links_externos_so_das_casas() {
        let aceito = |s: &str| link_externo_permitido(&Url::parse(s).unwrap());
        for ok in [
            "https://polymarket.com",
            "https://polymarket.com/event/abc?x=1#y",
            "https://kalshi.com/markets/KXNBA/KXNBA-25",
            "https://predict.fun/market/teste",
            "https://developers.predict.fun/",
            "https://sx.bet/",
            "https://WWW.Kalshi.COM/",
            "https://polymarket.com:443/",
        ] {
            assert!(aceito(ok), "{ok}");
        }
        for nao in [
            "http://polymarket.com/",
            "https://polymarket.com.evil.com/",
            "https://evilpolymarket.com/",
            "https://notsx.bet/",
            "https://polymarket.com:8443/",
            "https://user:senha@polymarket.com/",
            "https://user@kalshi.com/",
            "https://polymarket.com./",
            "https://.polymarket.com/",
            "https://example.com/?u=https://polymarket.com",
            "javascript:alert(1)",
            "file:///C:/Windows/System32/calc.exe",
            "ms-settings:privacy",
            "https://1.2.3.4/",
            "http://betty.localhost/dashboard.html",
            "blob:http://betty.localhost/abc",
        ] {
            assert!(!aceito(nao), "{nao}");
        }
    }
}
