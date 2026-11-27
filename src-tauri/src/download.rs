//! Downloads iniciados pela própria interface (exportação em CSV).
//!
//! A página gera o arquivo num `blob:` e clica num link com `download`. Sem
//! handler, o WebView2 não salva nada de forma confiável dentro do app; aqui o
//! download é aceito só quando vem de um blob da própria origem e o arquivo vai
//! para a pasta Downloads do usuário com o nome sugerido pela página (com
//! " (1)", " (2)"... se já existir um arquivo igual). Ao terminar, a página é
//! avisada para mostrar onde o arquivo ficou.

use std::path::{Path, PathBuf};

use tauri::webview::DownloadEvent;
use tauri::{Manager, Runtime, Url, Webview};

use crate::protocol;

const NOME_PADRAO: &str = "mybettys-exportacao.csv";

/// Handler do `on_download` da janela principal.
pub fn tratar<R: Runtime>(webview: Webview<R>, evento: DownloadEvent<'_>) -> bool {
    match evento {
        DownloadEvent::Requested { url, destination } => {
            if !download_permitido(&url) {
                eprintln!("[download] recusado: {}", url.scheme());
                return false;
            }
            let Some(pasta) = pasta_destino(&webview) else {
                eprintln!("[download] pasta Downloads indisponível");
                return false;
            };
            let nome = nome_seguro(destination.file_name().and_then(|n| n.to_str()));
            *destination = caminho_livre(&pasta, &nome);
            true
        }
        DownloadEvent::Finished { path, success, .. } => {
            let nome = path.as_deref().and_then(Path::file_name).and_then(|n| n.to_str()).unwrap_or("");
            let pasta = path.as_deref().and_then(Path::parent).map(|p| p.display().to_string()).unwrap_or_default();
            if !success {
                eprintln!("[download] falhou");
            }
            let aviso = serde_json::json!({ "ok": success, "nome": nome, "pasta": pasta });
            let _ = webview.eval(format!("window.avisarDownload && window.avisarDownload({aviso})"));
            true
        }
        _ => true,
    }
}

/// Só blobs criados pela própria interface (`blob:<origem do app>/<id>`).
pub fn download_permitido(url: &Url) -> bool {
    url.scheme() == "blob" && Url::parse(url.path()).is_ok_and(|interna| protocol::navegacao_permitida(&interna))
}

fn pasta_destino<R: Runtime>(webview: &Webview<R>) -> Option<PathBuf> {
    // Só em desenvolvimento: testes salvam numa pasta temporária.
    if cfg!(debug_assertions) {
        if let Some(p) = std::env::var_os("MYBETTYS_DOWNLOAD_DIR") {
            return Some(p.into());
        }
    }
    webview.app_handle().path().download_dir().ok()
}

/// Nome sugerido pela página, sem pastas nem caracteres proibidos no Windows.
pub fn nome_seguro(sugerido: Option<&str>) -> String {
    let limpo: String = sugerido
        .unwrap_or("")
        .chars()
        .map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c })
        .collect();
    let limpo = limpo.trim().trim_end_matches(['.', ' ']).to_string();
    let base = limpo.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reservado = matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((base.starts_with("COM") || base.starts_with("LPT")) && base.len() == 4 && base.as_bytes()[3].is_ascii_digit());
    if limpo.is_empty() || limpo.starts_with('.') || reservado { NOME_PADRAO.to_string() } else { limpo }
}

/// `pasta/nome`, ou `nome (n).ext` com o primeiro `n` que não existe.
pub fn caminho_livre(pasta: &Path, nome: &str) -> PathBuf {
    let primeiro = pasta.join(nome);
    if !primeiro.exists() {
        return primeiro;
    }
    let (base, ext) = match nome.rfind('.') {
        Some(i) if i > 0 => (&nome[..i], &nome[i..]),
        _ => (nome, ""),
    };
    (1..10_000)
        .map(|n| pasta.join(format!("{base} ({n}){ext}")))
        .find(|p| !p.exists())
        .unwrap_or(primeiro)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn so_blob_da_propria_origem() {
        let blob = format!("blob:{}", protocol::app_url("").unwrap().origin().ascii_serialization());
        assert!(download_permitido(&Url::parse(&format!("{blob}/3f2a-11")).unwrap()));
        assert!(!download_permitido(&Url::parse("blob:https://polymarket.com/3f2a").unwrap()));
        assert!(!download_permitido(&Url::parse("https://polymarket.com/arquivo.csv").unwrap()));
        assert!(!download_permitido(&Url::parse("data:text/csv,a;b").unwrap()));
    }

    #[test]
    fn nome_sem_pasta_nem_reservado() {
        assert_eq!(nome_seguro(Some("betty-apostas-all-2026-11-26.csv")), "betty-apostas-all-2026-11-26.csv");
        assert_eq!(nome_seguro(Some("..\\..\\x.csv")), NOME_PADRAO);
        assert_eq!(nome_seguro(Some("a/b:c.csv")), "a_b_c.csv");
        assert_eq!(nome_seguro(Some("CON.csv")), NOME_PADRAO);
        assert_eq!(nome_seguro(Some("com1")), NOME_PADRAO);
        assert_eq!(nome_seguro(Some("relatorio. ")), "relatorio");
        assert_eq!(nome_seguro(None), NOME_PADRAO);
    }

    #[test]
    fn nao_sobrescreve_arquivo_existente() {
        let pasta = tempfile::tempdir().unwrap();
        let nome = "betty.csv";
        assert_eq!(caminho_livre(pasta.path(), nome), pasta.path().join(nome));
        std::fs::write(pasta.path().join(nome), "x").unwrap();
        assert_eq!(caminho_livre(pasta.path(), nome), pasta.path().join("betty (1).csv"));
        std::fs::write(pasta.path().join("betty (1).csv"), "x").unwrap();
        assert_eq!(caminho_livre(pasta.path(), nome), pasta.path().join("betty (2).csv"));
    }
}
