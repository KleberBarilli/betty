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

/// A interface atual tem scripts, estilos e handlers inline, por isso ainda
/// precisa de 'unsafe-inline'. Todo o resto fica restrito à própria origem:
/// nenhuma consulta externa sai do navegador.
const CSP: &str = "default-src 'self'; script-src 'self' 'unsafe-inline'; \
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

pub async fn handle<R: Runtime>(app: &AppHandle<R>, request: Request<Vec<u8>>) -> Response<Vec<u8>> {
    let path = request.uri().path().to_owned();
    if path == "/health" || path.starts_with("/api/") {
        let inicio = std::time::Instant::now();
        let estado = app.state::<api::Estado>();
        let resposta = api::handle(&estado, request.method(), &path, request.uri().query(), request.body()).await;
        let corpo = match resposta.bruto {
            Some(bytes) => bytes,
            None => crate::js::para_vec(&resposta.body),
        };
        // Diagnóstico de desempenho, só em desenvolvimento: MYBETTYS_LOG_API=1.
        if cfg!(debug_assertions) && std::env::var_os("MYBETTYS_LOG_API").is_some() {
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
}
