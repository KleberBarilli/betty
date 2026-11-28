<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="design/marca/opcao-2/logo-escuro.svg">
    <img src="design/marca/opcao-2/logo-claro.svg" alt="Betty" width="260">
  </picture>
</p>

<p align="center">
  <a href="README.md">English</a> · <b>Português</b>
</p>

<p align="center">
  Posições, histórico e estatísticas da Polymarket, Kalshi, Predict.fun e SX Bet em um só app para Windows.
</p>

<p align="center">
  <img alt="Rust" src="https://img.shields.io/badge/Rust-000000?logo=rust&logoColor=white">
  <img alt="Tauri 2" src="https://img.shields.io/badge/Tauri_2-24C8D8?logo=tauri&logoColor=white">
  <img alt="SQLite" src="https://img.shields.io/badge/SQLite-003B57?logo=sqlite&logoColor=white">
  <img alt="JavaScript" src="https://img.shields.io/badge/JavaScript-F7DF1E?logo=javascript&logoColor=black">
  <img alt="HTML5" src="https://img.shields.io/badge/HTML5-E34F26?logo=html5&logoColor=white">
  <img alt="CSS3" src="https://img.shields.io/badge/CSS3-663399?logo=css&logoColor=white">
  <img alt="Windows 10 e 11" src="https://img.shields.io/badge/Windows_10_%7C_11-0078D4">
  <img alt="Licença MIT" src="https://img.shields.io/badge/licença-MIT-3ddc84">
</p>

## Instalar

Baixe o instalador em [Releases](https://github.com/KleberBarilli/betty/releases/latest) e execute. Ele instala sem perguntas e abre o Betty no fim. Depois, cadastre suas contas em **Conexões**. A interface está em português do Brasil (padrão) e em inglês: troque em **Configurações → Idioma**.

O instalador não tem assinatura digital, então o SmartScreen do Windows pode bloquear: escolha **Mais informações** e **Executar assim mesmo**.

## Suas contas

- O acesso é somente leitura. O Betty nunca envia ordens.
- As credenciais ficam no seu computador, cifradas com AES-256-GCM. A chave fica no Cofre de Credenciais do Windows.
- O app só se comunica com as APIs das quatro casas e com nós públicos de blockchain (Polygon e BSC). Não há servidor próprio nem telemetria.

## Desenvolvimento

| Parte | Tecnologia |
| --- | --- |
| Backend | Rust, em [`src-tauri/src`](src-tauri/src) |
| Janela | Tauri 2 com WebView2 |
| Banco | SQLite (rusqlite), em `%APPDATA%\com.betty.desktop\betty.db` |
| Interface | HTML, CSS e JavaScript puros, em [`ui`](ui) |
| Instalador | NSIS |

Requer Windows 10 ou 11, Rust 1.85 ou mais novo e a CLI do Tauri:

```powershell
cargo install tauri-cli --version "^2"
```

Rode dentro de `src-tauri`:

```powershell
cargo run
```

```powershell
cargo test
```

```powershell
cargo tauri build
```

O instalador sai em `src-tauri/target/release/bundle/nsis/`.

## Desempenho

O que já foi medido e otimizado, o que falta e como validar sem mudar o comportamento: [docs/desempenho.md](docs/desempenho.md).

## Licença

[MIT](LICENSE). Pode modificar e redistribuir; basta dar o crédito a este repositório.
