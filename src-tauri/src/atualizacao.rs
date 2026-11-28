//! Versão nova do Betty: consulta a última release do GitHub, compara com a
//! versão instalada e, quando a pessoa pede, baixa o instalador, confere o
//! SHA-256 publicado pelo GitHub e o executa. O instalador roda em modo
//! passivo (só a barra de progresso), fecha o Betty e o abre de novo no fim.
//!
//! Rotas (atendidas em `protocol.rs`, que tem o `AppHandle`):
//! - `GET  /api/atualizacao`          → `{ atual, ultima, disponivel, ... }`
//! - `POST /api/atualizacao/instalar` → baixa, confere, executa e fecha o app

use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

/// Repositório das releases. Só arquivos dele são baixados.
const REPOSITORIO: &str = "KleberBarilli/betty";
/// Nenhum instalador do Betty chega perto disso (hoje ~4 MB).
const TAMANHO_MAXIMO: u64 = 200 * 1024 * 1024;
/// A API do GitHub permite 60 consultas por hora sem token; a interface
/// consulta na abertura e a cada poucas horas, e o resultado fica guardado.
const VALIDADE_CONSULTA: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub versao: String,
    pub pagina: String,
    pub notas: String,
    pub publicada: String,
    pub instalador: Option<Instalador>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Instalador {
    pub nome: String,
    pub url: String,
    pub tamanho: u64,
    /// SHA-256 em hexadecimal minúsculo, do campo `digest` do GitHub.
    pub sha256: Option<String>,
}

static ULTIMA_CONSULTA: LazyLock<Mutex<Option<(Instant, Release)>>> = LazyLock::new(|| Mutex::new(None));

/// Versão instalada (a do `tauri.conf.json`). Em desenvolvimento,
/// `BETTY_VERSAO_FINGIDA` finge uma versão antiga para testar o aviso.
pub fn versao_instalada(versao_do_app: &str) -> String {
    if cfg!(debug_assertions)
        && let Ok(v) = std::env::var("BETTY_VERSAO_FINGIDA")
    {
        return v;
    }
    versao_do_app.to_owned()
}

/// `v1.2.3` / `1.2.3` → (1, 2, 3). Versões de pré-lançamento (`-beta`) e
/// textos fora do padrão não contam como versão nova.
pub fn analisar_versao(texto: &str) -> Option<(u64, u64, u64)> {
    let t = texto.trim().trim_start_matches(['v', 'V']);
    let mut partes = t.split('.');
    let a = partes.next()?.parse().ok()?;
    let b = partes.next()?.parse().ok()?;
    let c = partes.next()?.parse().ok()?;
    if partes.next().is_some() {
        return None;
    }
    Some((a, b, c))
}

pub fn e_mais_nova(candidata: &str, instalada: &str) -> bool {
    match (analisar_versao(candidata), analisar_versao(instalada)) {
        (Some(c), Some(i)) => c > i,
        _ => false,
    }
}

/// Lê a resposta de `GET /repos/{repo}/releases/latest`.
pub fn ler_release(v: &Value) -> Result<Release> {
    let tag = v.get("tag_name").and_then(Value::as_str).ok_or_else(|| anyhow!("release sem tag"))?;
    let texto = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
    let instalador = v.get("assets").and_then(Value::as_array).and_then(|lista| {
        lista.iter().find_map(|a| {
            let nome = a.get("name")?.as_str()?;
            if !nome.ends_with("_x64-setup.exe") {
                return None;
            }
            let url = a.get("browser_download_url")?.as_str()?;
            let sha256 = a
                .get("digest")
                .and_then(Value::as_str)
                .and_then(|d| d.strip_prefix("sha256:"))
                .filter(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
                .map(str::to_ascii_lowercase);
            Some(Instalador { nome: nome.to_owned(), url: url.to_owned(), tamanho: a.get("size")?.as_u64()?, sha256 })
        })
    });
    Ok(Release {
        versao: tag.trim_start_matches(['v', 'V']).to_owned(),
        pagina: texto("html_url"),
        notas: texto("body"),
        publicada: texto("published_at"),
        instalador,
    })
}

/// O instalador só pode vir das releases do próprio repositório, por https.
pub fn url_confiavel(url: &str) -> bool {
    let Ok(u) = url::Url::parse(url) else { return false };
    u.scheme() == "https"
        && u.host_str() == Some("github.com")
        && u.port().is_none()
        && u.username().is_empty()
        && u.path().starts_with(&format!("/{REPOSITORIO}/releases/download/"))
}

/// Nome de arquivo seguro para gravar na pasta temporária.
fn nome_seguro(nome: &str) -> bool {
    !nome.is_empty() && nome.len() <= 120 && nome.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)) && !nome.starts_with('.')
}

async fn buscar_ultima_release(forcar: bool) -> Result<Release> {
    if !forcar
        && let Ok(g) = ULTIMA_CONSULTA.lock()
        && let Some((em, r)) = g.as_ref()
        && em.elapsed() < VALIDADE_CONSULTA
    {
        return Ok(r.clone());
    }
    let url = format!("https://api.github.com/repos/{REPOSITORIO}/releases/latest");
    let resposta = crate::net::CLIENTE
        .get(&url)
        .header("accept", "application/vnd.github+json")
        .header("user-agent", "Betty-desktop")
        .header("x-github-api-version", "2022-11-28")
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .context("sem resposta do GitHub")?;
    let status = resposta.status();
    if !status.is_success() {
        bail!("GitHub respondeu HTTP {}", status.as_u16());
    }
    let v: Value = serde_json::from_str(&resposta.text().await?).context("resposta do GitHub ilegível")?;
    let release = ler_release(&v)?;
    if let Ok(mut g) = ULTIMA_CONSULTA.lock() {
        *g = Some((Instant::now(), release.clone()));
    }
    Ok(release)
}

/// `GET /api/atualizacao`.
pub async fn consultar(versao_do_app: &str, forcar: bool) -> Value {
    let atual = versao_instalada(versao_do_app);
    match buscar_ultima_release(forcar).await {
        Ok(r) => {
            let disponivel = e_mais_nova(&r.versao, &atual) && r.instalador.is_some();
            json!({
                "atual": atual,
                "ultima": r.versao,
                "disponivel": disponivel,
                "pagina": r.pagina,
                "notas": r.notas,
                "publicada": r.publicada,
                "tamanho": r.instalador.as_ref().map(|i| i.tamanho),
            })
        }
        Err(e) => {
            eprintln!("[atualização] consulta falhou: {e:#}");
            json!({ "atual": atual, "disponivel": false, "erro": "Não foi possível consultar o GitHub." })
        }
    }
}

/// Baixa o instalador da última release, confere tamanho e SHA-256 e grava
/// numa pasta temporária. Devolve o caminho do arquivo.
pub async fn baixar(versao_do_app: &str) -> Result<std::path::PathBuf> {
    let atual = versao_instalada(versao_do_app);
    let release = buscar_ultima_release(true).await?;
    if !e_mais_nova(&release.versao, &atual) {
        bail!("o Betty já está na versão mais recente");
    }
    let inst = release.instalador.ok_or_else(|| anyhow!("a release não tem instalador para Windows"))?;
    if !url_confiavel(&inst.url) || !nome_seguro(&inst.nome) {
        bail!("endereço do instalador fora das releases do Betty");
    }
    let esperado = inst.sha256.ok_or_else(|| anyhow!("o GitHub não publicou o SHA-256 do instalador"))?;
    if inst.tamanho > TAMANHO_MAXIMO {
        bail!("instalador grande demais");
    }

    let resposta = crate::net::CLIENTE
        .get(&inst.url)
        .header("user-agent", "Betty-desktop")
        .timeout(Duration::from_secs(10 * 60))
        .send()
        .await
        .context("download do instalador")?;
    if !resposta.status().is_success() {
        bail!("download do instalador: HTTP {}", resposta.status().as_u16());
    }
    let bytes = resposta.bytes().await.context("download do instalador")?;
    if bytes.len() as u64 != inst.tamanho {
        bail!("instalador incompleto ({} de {} bytes)", bytes.len(), inst.tamanho);
    }
    let obtido = hex::encode(ring::digest::digest(&ring::digest::SHA256, &bytes));
    if obtido != esperado {
        bail!("o SHA-256 do instalador não confere com o publicado no GitHub");
    }

    let pasta = std::env::temp_dir().join("betty-atualizacao");
    let _ = std::fs::remove_dir_all(&pasta);
    std::fs::create_dir_all(&pasta).context("pasta temporária")?;
    let caminho = pasta.join(&inst.nome);
    std::fs::write(&caminho, &bytes).context("gravar o instalador")?;
    Ok(caminho)
}

/// Executa o instalador já conferido. `/UPDATE` é o mesmo sinal que o
/// atualizador do Tauri passa: mantém os atalhos como a pessoa deixou. O
/// instalador é passivo, fecha o Betty se ainda estiver aberto e o reabre.
pub fn executar(caminho: &std::path::Path) -> Result<()> {
    std::process::Command::new(caminho).arg("/UPDATE").spawn().context("não foi possível abrir o instalador")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compara_versoes() {
        assert!(e_mais_nova("0.2.0", "0.1.0"));
        assert!(e_mais_nova("v0.1.1", "0.1.0"));
        assert!(e_mais_nova("1.0.0", "0.9.9"));
        assert!(e_mais_nova("0.10.0", "0.9.0"), "compara números, não texto");
        assert!(!e_mais_nova("0.1.0", "0.1.0"));
        assert!(!e_mais_nova("0.0.9", "0.1.0"));
        assert!(!e_mais_nova("0.2.0-beta", "0.1.0"), "pré-lançamento não conta");
        assert!(!e_mais_nova("lixo", "0.1.0"));
        assert!(!e_mais_nova("0.2", "0.1.0"));
    }

    #[test]
    fn le_a_release_do_github() {
        let v = json!({
            "tag_name": "v0.2.0",
            "html_url": "https://github.com/KleberBarilli/betty/releases/tag/v0.2.0",
            "body": "Novidades",
            "published_at": "2026-12-01T10:00:00Z",
            "assets": [
                { "name": "notas.txt", "browser_download_url": "https://github.com/KleberBarilli/betty/releases/download/v0.2.0/notas.txt", "size": 10 },
                {
                    "name": "Betty_0.2.0_x64-setup.exe",
                    "browser_download_url": "https://github.com/KleberBarilli/betty/releases/download/v0.2.0/Betty_0.2.0_x64-setup.exe",
                    "size": 4136815,
                    "digest": "sha256:30F6F717D02FF2A3F6569660829D73A4F56076DE7ED198C5AB00868D90F16C4F"
                }
            ]
        });
        let r = ler_release(&v).unwrap();
        assert_eq!(r.versao, "0.2.0");
        let i = r.instalador.unwrap();
        assert_eq!(i.nome, "Betty_0.2.0_x64-setup.exe");
        assert_eq!(i.tamanho, 4136815);
        assert_eq!(i.sha256.as_deref(), Some("30f6f717d02ff2a3f6569660829d73a4f56076de7ed198c5ab00868d90f16c4f"));

        let sem = ler_release(&json!({ "tag_name": "v0.2.0", "assets": [] })).unwrap();
        assert!(sem.instalador.is_none());
        let digest_ruim = ler_release(&json!({ "tag_name": "v1.0.0", "assets": [{ "name": "B_x64-setup.exe", "browser_download_url": "x", "size": 1, "digest": "md5:abc" }] })).unwrap();
        assert_eq!(digest_ruim.instalador.unwrap().sha256, None);
    }

    #[test]
    fn so_baixa_das_releases_do_betty() {
        assert!(url_confiavel("https://github.com/KleberBarilli/betty/releases/download/v0.2.0/Betty_0.2.0_x64-setup.exe"));
        for nao in [
            "http://github.com/KleberBarilli/betty/releases/download/v0.2.0/a.exe",
            "https://github.com/outro/betty/releases/download/v0.2.0/a.exe",
            "https://github.com.evil.com/KleberBarilli/betty/releases/download/v0.2.0/a.exe",
            "https://user@github.com/KleberBarilli/betty/releases/download/v0.2.0/a.exe",
            "https://github.com:8443/KleberBarilli/betty/releases/download/v0.2.0/a.exe",
            "https://evil.com/KleberBarilli/betty/releases/download/v0.2.0/a.exe",
        ] {
            assert!(!url_confiavel(nao), "{nao}");
        }
        assert!(nome_seguro("Betty_0.2.0_x64-setup.exe"));
        assert!(!nome_seguro("..\\..\\x.exe"));
        assert!(!nome_seguro("a/b.exe"));
        assert!(!nome_seguro(".exe"));
    }

    /// Consulta real ao GitHub: `cargo test -- --ignored consulta_o_github`.
    #[test]
    #[ignore]
    fn consulta_o_github() {
        let r = tauri::async_runtime::block_on(buscar_ultima_release(true)).unwrap();
        eprintln!("{r:?}");
        let i = r.instalador.expect("release com instalador");
        assert!(url_confiavel(&i.url));
        assert!(i.sha256.is_some());
    }

    /// Download real e conferência do SHA-256, sem executar o instalador:
    /// `cargo test -- --ignored baixa_e_confere`.
    #[test]
    #[ignore]
    fn baixa_e_confere_o_instalador() {
        let caminho = tauri::async_runtime::block_on(baixar("0.0.1")).unwrap();
        assert!(caminho.metadata().unwrap().len() > 1_000_000);
        let _ = std::fs::remove_dir_all(caminho.parent().unwrap());
        let r = tauri::async_runtime::block_on(baixar("999.0.0"));
        assert!(r.is_err(), "sem versão mais nova, não baixa");
    }
}
