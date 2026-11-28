<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="design/marca/opcao-2/logo-escuro.svg">
    <img src="design/marca/opcao-2/logo-claro.svg" alt="Betty" width="260">
  </picture>
</p>

<p align="center">
  <b>English</b> · <a href="README.pt-BR.md">Português</a>
</p>

<p align="center">
  Positions, history and stats from Polymarket, Kalshi, Predict.fun and SX Bet in one Windows app.
</p>

<p align="center">
  <img alt="Rust" src="https://img.shields.io/badge/Rust-000000?logo=rust&logoColor=white">
  <img alt="Tauri 2" src="https://img.shields.io/badge/Tauri_2-24C8D8?logo=tauri&logoColor=white">
  <img alt="SQLite" src="https://img.shields.io/badge/SQLite-003B57?logo=sqlite&logoColor=white">
  <img alt="JavaScript" src="https://img.shields.io/badge/JavaScript-F7DF1E?logo=javascript&logoColor=black">
  <img alt="HTML5" src="https://img.shields.io/badge/HTML5-E34F26?logo=html5&logoColor=white">
  <img alt="CSS3" src="https://img.shields.io/badge/CSS3-663399?logo=css&logoColor=white">
  <img alt="Windows 10 and 11" src="https://img.shields.io/badge/Windows_10_%7C_11-0078D4">
  <img alt="MIT license" src="https://img.shields.io/badge/license-MIT-3ddc84">
</p>

## Install

Download the installer from [Releases](https://github.com/KleberBarilli/betty/releases/latest) and run it. It installs without prompts and opens Betty when done. The interface comes in Brazilian Portuguese (default) and English: switch under **Configurações → Idioma**. Then add your accounts under **Connections** (**Conexões**).

The installer is not code-signed, so Windows SmartScreen may block it: choose **More info**, then **Run anyway**.

## Your accounts

- Access is read-only. Betty never places orders.
- Credentials stay on your machine, encrypted with AES-256-GCM. The key lives in the Windows Credential Manager.
- The app only talks to the four platforms' APIs and to public blockchain nodes (Polygon and BSC). There is no backend server and no telemetry.

## Development

| Part | Tech |
| --- | --- |
| Backend | Rust, in [`src-tauri/src`](src-tauri/src) |
| Window | Tauri 2 with WebView2 |
| Database | SQLite (rusqlite), at `%APPDATA%\com.betty.desktop\betty.db` |
| Interface | Plain HTML, CSS and JavaScript, in [`ui`](ui) |
| Installer | NSIS |

Requires Windows 10 or 11, Rust 1.85 or newer and the Tauri CLI:

```powershell
cargo install tauri-cli --version "^2"
```

Run these inside `src-tauri`:

```powershell
cargo run
```

```powershell
cargo test
```

```powershell
cargo tauri build
```

The installer is written to `src-tauri/target/release/bundle/nsis/`.

## Performance

What has been measured and optimized, what is left, and how to validate without changing behavior (in Portuguese): [docs/desempenho.md](docs/desempenho.md).

## License

[MIT](LICENSE). You can modify and redistribute it; just credit this repository.
