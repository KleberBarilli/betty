//! Rotas `/api/*` do Betty, com o mesmo contrato JSON do `server.js`.
//!
//! Rotas que não existem no app desktop respondem 501.

use std::sync::Arc;

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Map, Value, json};
use tauri::http::{Method, StatusCode};

use crate::crypto::MasterKey;
use crate::db::{self, Db, UsuarioLocal};
use crate::kalshi::{self, EstadoKalshi};
use crate::poly::{self, EstadoPoly};
use crate::{history, js, ledger, poly_datas, tracked};

/// Casas na ordem do `CASAS_HISTORICO` do Electron.
pub const CASAS: [&str; 4] = ["polymarket", "kalshi", "predict", "sxbet"];

/// Limites do `express.json()` do backend antigo: 2 MB em geral e 8 MB nas
/// rotas que recebem lotes do histórico.
const LIMITE_CORPO: usize = 2 * 1024 * 1024;
const LIMITE_LEDGER: usize = 8 * 1024 * 1024;

pub struct Estado {
    pub db: Db,
    pub chave: Arc<MasterKey>,
    pub usuario: UsuarioLocal,
    pub kalshi: Arc<EstadoKalshi>,
    pub poly: Arc<EstadoPoly>,
    pub tracked: Arc<tracked::EstadoTracked>,
}

pub struct ApiResponse {
    pub status: StatusCode,
    pub body: Value,
    /// Corpo já serializado (respostas grandes montadas a partir do JSON bruto
    /// do ledger). Quando existe, substitui `body`.
    pub bruto: Option<Vec<u8>>,
}

/// Resultado de uma rota: JSON montado aqui ou bytes já serializados.
pub enum Corpo {
    Json(Value),
    Bytes(Vec<u8>),
}

impl From<Value> for Corpo {
    fn from(v: Value) -> Self {
        Corpo::Json(v)
    }
}

impl From<Vec<u8>> for Corpo {
    fn from(b: Vec<u8>) -> Self {
        Corpo::Bytes(b)
    }
}

pub async fn handle(estado: &Estado, method: &Method, path: &str, query: Option<&str>, corpo: &[u8]) -> ApiResponse {
    use history as h;

    let resultado = match (method.as_str(), path) {
        ("GET", "/health") => Ok(ok(json!({ "ok": true, "service": "betty", "engine": "rust" }))),
        ("GET", "/api/me") => me(estado).await,
        // No app desktop não há sessão: sair só volta para a tela inicial.
        ("POST", "/api/auth/logout") => Ok(ok(json!({ "ok": true }))),
        ("GET", "/api/settings") => ler_settings(estado).await,
        ("POST", "/api/settings") => match ler_json(corpo, LIMITE_CORPO) {
            Ok(body) => salvar_settings(estado, body).await,
            Err(resposta) => Ok(resposta),
        },
        ("GET", "/api/kalshi/test") => Ok(kalshi::testar(estado).await),
        ("GET", "/api/kalshi/balance") => Ok(kalshi::saldo_rota(estado).await),
        ("GET", "/api/kalshi/positions") => Ok(kalshi::posicoes(estado).await),
        ("GET", "/api/kalshi/settlements") => Ok(kalshi::liquidacoes(estado).await),
        ("POST", "/api/connections/kalshi") => match ler_json(corpo, LIMITE_CORPO) {
            Ok(body) => com_conexoes(estado, kalshi::conectar(estado, &body).await, "Não foi possível proteger as credenciais da Kalshi.").await,
            Err(resposta) => Ok(resposta),
        },
        ("DELETE", "/api/connections/kalshi") => com_conexoes(estado, kalshi::desconectar(estado).await, ERRO_GENERICO).await,
        ("GET", "/api/polymarket/balance") => Ok(poly::saldo(estado).await),
        ("GET", p) if p.starts_with("/api/polymarket/") => Ok(poly::proxy(estado, false, &p["/api/polymarket/".len()..], query).await),
        ("GET", p) if p.starts_with("/api/polymarket-gamma/") => {
            Ok(poly::proxy(estado, true, &p["/api/polymarket-gamma/".len()..], query).await)
        }
        ("POST", "/api/connections/polymarket") => match ler_json(corpo, LIMITE_CORPO) {
            Ok(body) => com_conexoes(estado, poly::conectar(estado, &body).await, "Não foi possível proteger a wallet da Polymarket no servidor.").await,
            Err(resposta) => Ok(resposta),
        },
        ("DELETE", "/api/connections/polymarket") => com_conexoes(estado, poly::desconectar(estado).await, ERRO_GENERICO).await,
        ("POST", "/api/connections/polymarket/preview") => match ler_json(corpo, LIMITE_CORPO) {
            Ok(body) => Ok(poly::preview(estado, &body).await),
            Err(resposta) => Ok(resposta),
        },
        ("POST", "/api/polymarket-metadata/cache") => match ler_json(corpo, LIMITE_LEDGER) {
            Ok(body) => Ok(poly::cache_metadados(estado, &body).await),
            Err(resposta) => Ok(resposta),
        },
        ("POST", p @ ("/api/history/closed" | "/api/history/closed/delta" | "/api/history/closed/lookup" | "/api/history/closed/metadata"
            | "/api/history/closed/index" | "/api/history/closed/index/titles" | "/api/history/closed/index/versions"
            | "/api/history/closed/recover-pnl-dates")) => {
            let limite = if matches!(p, "/api/history/closed/metadata" | "/api/history/closed/recover-pnl-dates" | "/api/history/closed/index/versions") {
                LIMITE_CORPO
            } else {
                LIMITE_LEDGER
            };
            match ler_json(corpo, limite) {
                Ok(body) => escrita_historico(estado, p, body).await,
                Err(resposta) => Ok(resposta),
            }
        }
        ("GET", "/api/history/closed") => {
            // Como o Node: a falha na atualização dos metadados não impede a leitura.
            if let Err(e) = kalshi::atualizar_modalidades_no_ledger(estado).await {
                eprintln!("[api] metadados Kalshi do ledger: {e:#}");
            }
            historico(estado, query, ERRO_GENERICO, "", |c, u, _| h::ledger(c, u).map(|b| Some(b.into()))).await
        }
        ("GET", "/api/history/closed/export") => {
            historico(estado, query, "Não foi possível preparar a exportação.", "", |c, u, _| {
                h::exportar(c, u).map(|b| Some(b.into()))
            })
            .await
        }
        ("GET", "/api/history/closed/index/status") => {
            historico(estado, query, ERRO_GENERICO, "", |c, u, _| h::status_indice(c, u).map(|v| Some(v.into()))).await
        }
        ("GET", "/api/history/closed/index/pending") => {
            historico(estado, query, "Não foi possível consultar as pendências do histórico.", "Cursor inválido.", |c, u, q| {
                h::pendentes_indice(c, u, q).map(|b| b.map(Corpo::from))
            })
            .await
        }
        ("GET", "/api/history/closed/query") => {
            historico(estado, query, "Não foi possível consultar o histórico fechado.", "", |c, u, q| {
                h::consultar(c, u, q).map(|b| Some(b.into()))
            })
            .await
        }
        ("GET", "/api/history/closed/layout") => {
            historico(estado, query, "Não foi possível medir o histórico.", "", |c, u, q| h::layout(c, u, q).map(|v| Some(v.into())))
                .await
        }
        ("GET", "/api/history/closed/stats") => {
            historico(estado, query, "Não foi possível calcular as estatísticas.", "", |c, u, q| {
                h::stats(c, u, q).map(|v| Some(v.into()))
            })
            .await
        }
        ("GET", "/api/history/closed/calendar") => {
            historico(
                estado,
                query,
                "Não foi possível carregar o calendário.",
                "Selecione um mês válido (AAAA-MM).",
                |c, u, q| h::calendario(c, u, q).map(|v| v.map(Corpo::from)),
            )
            .await
        }
        ("GET", "/api/history/closed/pnl") => {
            historico(estado, query, "Não foi possível consultar o PnL.", "", |c, u, q| h::pnl(c, u, q).map(|v| Some(v.into()))).await
        }
        (m, p) if p.starts_with("/api/connections/") || p.starts_with("/api/tracked/") => rastreadas(estado, m, p, corpo).await,
        _ => Ok(nao_migrada(method, path)),
    };
    resultado.unwrap_or_else(|e| {
        // O detalhe fica só no log; a interface recebe uma mensagem genérica.
        eprintln!("[api] {method} {path}: {e:#}");
        erro(StatusCode::INTERNAL_SERVER_ERROR, "Erro interno do Betty.")
    })
}

type Resultado = anyhow::Result<ApiResponse>;

const ERRO_GENERICO: &str = "Erro interno do Betty.";

/// Um segmento de rota (`:platform`), sem barra.
fn segmento<'a>(p: &'a str, prefixo: &str, sufixo: &str) -> Option<&'a str> {
    let s = p.strip_prefix(prefixo)?.strip_suffix(sufixo)?;
    (!s.is_empty() && !s.contains('/')).then_some(s)
}

/// Predict.fun e SX Bet: conexão, teste, sincronização e saldos.
async fn rastreadas(estado: &Estado, metodo: &str, p: &str, corpo: &[u8]) -> Resultado {
    if metodo == "GET" && p == "/api/tracked/predict/balance" {
        return Ok(tracked::saldo_predict(estado).await);
    }
    if metodo == "GET" && p == "/api/tracked/sxbet/balance" {
        return Ok(tracked::saldo_sxbet(estado).await);
    }
    let conexao = segmento(p, "/api/connections/", "");
    match (metodo, conexao) {
        ("POST", Some(casa)) => {
            return match ler_json(corpo, LIMITE_CORPO) {
                Ok(body) => com_conexoes(estado, tracked::conectar(estado, casa, &body).await, ERRO_GENERICO).await,
                Err(resposta) => Ok(resposta),
            };
        }
        ("DELETE", Some(casa)) => return com_conexoes(estado, tracked::desconectar(estado, casa).await, ERRO_GENERICO).await,
        _ => {}
    }
    if metodo == "GET" {
        if let Some(casa) = segmento(p, "/api/connections/", "/test") {
            return Ok(tracked::testar(estado, casa).await);
        }
    }
    if metodo == "POST" {
        if let Some(casa) = segmento(p, "/api/tracked/", "/sync") {
            return match ler_json(corpo, LIMITE_CORPO) {
                Ok(_) => Ok(tracked::sincronizar(estado, casa).await),
                Err(resposta) => Ok(resposta),
            };
        }
    }
    let m: Method = metodo.parse().unwrap_or(Method::GET);
    Ok(nao_migrada(&m, p))
}

/// Executa uma consulta de histórico no banco. `Ok(None)` vira 400 com
/// `msg_invalida`; um erro vira 500 com `msg_erro`, como no `server.js`.
async fn historico<F>(estado: &Estado, query: Option<&str>, msg_erro: &'static str, msg_invalida: &'static str, f: F) -> Resultado
where
    F: FnOnce(&mut Connection, &str, &history::Consulta) -> anyhow::Result<Option<Corpo>> + Send + 'static,
{
    let consulta = history::ler_consulta(query);
    let user_id = estado.usuario.id.clone();
    match estado.db.call(move |c| f(c, &user_id, &consulta)).await {
        Ok(Some(Corpo::Json(corpo))) => Ok(ok(corpo)),
        Ok(Some(Corpo::Bytes(b))) => Ok(ApiResponse { status: StatusCode::OK, body: Value::Null, bruto: Some(b) }),
        Ok(None) => Ok(erro(StatusCode::BAD_REQUEST, msg_invalida)),
        Err(e) => {
            eprintln!("[api] histórico: {e:#}");
            Ok(erro(StatusCode::INTERNAL_SERVER_ERROR, msg_erro))
        }
    }
}

/// Acrescenta `connections` às respostas de conexão bem-sucedidas.
async fn com_conexoes(estado: &Estado, r: anyhow::Result<ApiResponse>, msg_erro: &str) -> Resultado {
    let mut r = match r {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[api] conexão: {e:#}");
            return Ok(erro(StatusCode::INTERNAL_SERVER_ERROR, msg_erro));
        }
    };
    if r.status == StatusCode::OK {
        let chave = estado.chave.clone();
        let user_id = estado.usuario.id.clone();
        r.body["connections"] = estado.db.call(move |c| conexoes_seguras(c, &chave, &user_id)).await?;
    }
    Ok(r)
}

/// Rotas que gravam no histórico (ledger e índice).
async fn escrita_historico(estado: &Estado, rota: &str, body: Value) -> Resultado {
    let user = estado.usuario.id.clone();
    let lista = |k: &str| body.get(k).filter(|v| v.is_array()).cloned().unwrap_or_else(|| json!([]));
    let (resultado, msg): (anyhow::Result<ApiResponse>, &str) = match rota {
        "/api/history/closed" => {
            let linhas = lista("fechados");
            let r = estado
                .db
                .call(move |c| {
                    ledger::gravar(c, &user, linhas.as_array().cloned().unwrap_or_default(), false)?;
                    history::ledger_legado(c, &user)
                })
                .await
                .map(|b| ApiResponse { status: StatusCode::OK, body: Value::Null, bruto: Some(b) });
            (r, "Não foi possível salvar o histórico persistente.")
        }
        "/api/history/closed/delta" => (delta(estado, body).await, "Não foi possível sincronizar o histórico persistente."),
        "/api/history/closed/recover-pnl-dates" => {
            let r = poly_datas::recuperar(&estado.db, &user, None)
                .await
                .map(|r| ok(json!({ "ok": true, "saved": r.salvas, "pending": r.pendentes })));
            (r, "Não foi possível recuperar as datas agora.")
        }
        "/api/history/closed/lookup" => {
            let r = estado
                .db
                .call(move |c| ledger::assinaturas(c, &user, body.get("entries")))
                .await
                .map(|e| ok(json!({ "ok": true, "count": e.len(), "entries": e })));
            (r, "Não foi possível consultar o histórico persistente.")
        }
        "/api/history/closed/metadata" => {
            let r = estado.db.call(move |c| ledger::metadados(c, &user, body.get("keys"))).await.map(|e| ok(json!({ "ok": true, "entries": e })));
            (r, "Não foi possível consultar os metadados.")
        }
        "/api/history/closed/index/versions" => {
            let r = estado
                .db
                .call(move |c| {
                    db::salvar_versoes_indice(c, &user, body.get("versions"))?;
                    Ok(json!({ "ok": true, "versions": db::versoes_indice(c, &user)? }))
                })
                .await
                .map(ok);
            (r, "Não foi possível registrar a versão do índice.")
        }
        "/api/history/closed/index" => {
            let r = estado
                .db
                .call(move |c| {
                    let (recebidas, gravadas) = ledger::salvar_indice(c, &user, body.get("entries"))?;
                    let indexadas: i64 = c.query_row("SELECT COUNT(*) FROM closed_bets_index WHERE user_id=?", [&user], |r| r.get(0))?;
                    Ok(json!({ "ok": true, "received": recebidas, "saved": gravadas, "indexed": indexadas }))
                })
                .await
                .map(ok);
            (r, "Não foi possível indexar o histórico fechado.")
        }
        _ => {
            let r = estado
                .db
                .call(move |c| ledger::atualizar_titulos(c, &user, body.get("entries")))
                .await
                .map(|(recebidas, atualizadas)| ok(json!({ "ok": true, "received": recebidas, "updated": atualizadas })));
            (r, "Não foi possível atualizar os títulos do histórico fechado.")
        }
    };
    Ok(resultado.unwrap_or_else(|e| {
        eprintln!("[api] {rota}: {e:#}");
        erro(StatusCode::INTERNAL_SERVER_ERROR, msg)
    }))
}

/// `POST /api/history/closed/delta`: grava só apostas novas ou alteradas e
/// devolve as linhas do lote na forma canônica.
async fn delta(estado: &Estado, body: Value) -> anyhow::Result<ApiResponse> {
    let user = estado.usuario.id.clone();
    let linhas = body.get("fechados").and_then(Value::as_array).cloned().unwrap_or_default();
    let versao = body.get("titleVersion").cloned();
    let u = user.clone();
    let g = estado.db.call(move |c| ledger::gravar(c, &u, linhas, true)).await?;
    // Só perdas resolvidas sem data podem precisar da recuperação (o mesmo filtro de dentro).
    let entradas = g
        .processadas
        .iter()
        .filter(|b| crate::audit::resolved_loss(b) && crate::audit::timestamp_ms(b.get("_resolvedTime")).is_none_or(|v| v == 0.0))
        .map(|b| (ledger::chave_ledger(b), b.clone()))
        .collect();
    let recuperadas = poly_datas::recuperar(&estado.db, &user, Some(entradas)).await?;
    let mut processadas = g.processadas;
    for linha in &mut processadas {
        if let Some(data) = recuperadas.datas.get(&ledger::chave_ledger(linha)) {
            let mut com_data = linha.as_object().cloned().unwrap_or_default();
            com_data.insert("_resolvedTime".into(), json!(data));
            let protegida = crate::audit::protect_closed_date(Some(linha), Value::Object(com_data));
            // `Object.assign(row, ...)`: chaves existentes ficam na mesma posição.
            if let (Some(destino), Some(origem)) = (linha.as_object_mut(), protegida.as_object()) {
                for (k, v) in origem {
                    destino.insert(k.clone(), v.clone());
                }
            }
        }
    }
    let (recebidas, gravadas, inalteradas) = (g.recebidas, g.gravadas, g.inalteradas);
    let u = user.clone();
    let corpo = estado
        .db
        .call(move |c| {
            let total: i64 = c.query_row("SELECT COUNT(*) FROM closed_bets_ledger WHERE user_id=?", [&u], |r| r.get(0))?;
            let fechados = ledger::preparar_titulos_kalshi(c, &u, processadas, versao.as_ref())?;
            Ok(json!({ "ok": true, "count": total, "received": recebidas, "saved": gravadas, "unchanged": inalteradas, "fechados": fechados }))
        })
        .await?;
    Ok(ok(corpo))
}

async fn me(estado: &Estado) -> Resultado {
    let usuario = estado.usuario.clone();
    let chave = estado.chave.clone();
    let user_id = usuario.id.clone();
    let conexoes = estado.db.call(move |c| conexoes_seguras(c, &chave, &user_id)).await?;
    Ok(ok(json!({ "ok": true, "user": usuario, "connections": conexoes })))
}

/// `safeConnection()`: diz quais casas estão conectadas sem devolver nenhum
/// segredo. A wallet aparece só mascarada.
fn conexoes_seguras(c: &Connection, chave: &MasterKey, user_id: &str) -> anyhow::Result<Value> {
    type Linha = (Option<String>, Option<String>, Option<String>, Option<String>);
    let linha: Option<Linha> = c
        .query_row(
            "SELECT polymarket_wallet_enc, kalshi_api_key_enc, kalshi_private_key_enc, updated_at FROM connections WHERE user_id=?",
            [user_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let (wallet_enc, kalshi_key, kalshi_priv, updated_at) = linha.unwrap_or((None, None, None, None));
    let preenchido = |v: &Option<String>| v.as_deref().is_some_and(|s| !s.is_empty());

    let conectadas: Vec<String> = {
        let mut stmt = c.prepare("SELECT platform FROM tracked_connections WHERE user_id=? AND credentials_enc IS NOT NULL")?;
        let linhas = stmt.query_map([user_id], |r| r.get(0))?;
        linhas.collect::<rusqlite::Result<_>>()?
    };
    let historico_rastreado = c
        .query_row(
            "SELECT 1 FROM tracked_connections c WHERE c.user_id=? AND c.credentials_enc IS NULL AND EXISTS (
               SELECT 1 FROM closed_bets_ledger l WHERE l.user_id=c.user_id AND l.platform=c.platform
               AND l.bet_key GLOB c.account_scope || ':*') LIMIT 1",
            [user_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some();

    let dica = wallet_enc
        .as_deref()
        .filter(|v| !v.is_empty())
        .and_then(|v| chave.decrypt(v).ok())
        .and_then(|w| dica_wallet(&w));

    Ok(json!({
        "polymarket": preenchido(&wallet_enc),
        "polymarketWalletHint": dica,
        "kalshi": preenchido(&kalshi_key) && preenchido(&kalshi_priv),
        "predict": conectadas.iter().any(|p| p == "predict"),
        "sxbet": conectadas.iter().any(|p| p == "sxbet"),
        "trackedHistory": historico_rastreado,
        "updatedAt": updated_at
    }))
}

/// `walletHint()`: `0x1234…abcd`.
fn dica_wallet(w: &str) -> Option<String> {
    let valida = w.len() == 42 && w.starts_with("0x") && w[2..].bytes().all(|b| b.is_ascii_hexdigit());
    valida.then(|| format!("{}…{}", &w[..6], &w[38..]))
}

async fn ler_settings(estado: &Estado) -> Resultado {
    let user_id = estado.usuario.id.clone();
    let config = estado
        .db
        .call(move |c| {
            let linha = c
                .query_row(
                    "SELECT stake_unidade, stake_regras, stake_por_casa, closed_range_start, closed_range_end, closed_range_by_platform \
                     FROM settings WHERE user_id=?",
                    [&user_id],
                    |r| {
                        Ok((
                            r.get::<_, f64>(0)?,
                            r.get::<_, Option<String>>(1)?,
                            r.get::<_, Option<String>>(2)?,
                            r.get::<_, Option<String>>(3)?,
                            r.get::<_, Option<String>>(4)?,
                            r.get::<_, Option<String>>(5)?,
                        ))
                    },
                )
                .optional()?;
            Ok(linha)
        })
        .await?;
    let Some((stake, regras, por_casa, inicio, fim, periodo)) = config else {
        return Ok(ok(json!({ "ok": true, "config": null })));
    };
    // `JSON.parse(x || padrão)`, mantendo o padrão quando o texto é inválido.
    let parse = |texto: Option<String>, padrao: Value| -> Value {
        match texto.filter(|t| !t.is_empty()) {
            Some(t) => serde_json::from_str(&t).unwrap_or(padrao),
            None => padrao,
        }
    };
    Ok(ok(json!({
        "ok": true,
        "config": {
            "stakeUnidade": js::numero_json(stake),
            "stakeRegras": parse(regras, json!([])),
            "stakePorCasa": parse(por_casa, Value::Null),
            "periodoFechadasInicio": inicio,
            "periodoFechadasFim": fim,
            "periodoFechadasPorCasa": parse(periodo, Value::Null)
        }
    })))
}

/// Mesmas regras do `POST /api/settings` do Electron.
async fn salvar_settings(estado: &Estado, body: Value) -> Resultado {
    let stake = js::number(body.get("stakeUnidade"));
    if !stake.is_finite() || stake <= 0.0 {
        return Ok(erro(StatusCode::BAD_REQUEST, "Configuração inválida."));
    }
    let regras_globais = filtrar_regras(body.get("stakeRegras"));

    let recebido_casa = body.get("stakePorCasa").filter(|v| v.is_object());
    let mut stake_por_casa = Map::new();
    for casa in CASAS {
        let raw = recebido_casa.and_then(|o| o.get(casa));
        let unidade = js::number(raw.and_then(|r| r.get("unidade")));
        let regras = match raw.and_then(|r| r.get("regras")) {
            Some(Value::Array(_)) => filtrar_regras(raw.and_then(|r| r.get("regras"))),
            _ => regras_globais.clone(),
        };
        let unidade = if unidade.is_finite() && unidade > 0.0 { unidade } else { stake };
        stake_por_casa.insert(casa.into(), json!({ "unidade": js::numero_json(unidade), "regras": regras }));
    }

    let normalizar_data = |v: Option<&Value>| -> String {
        let texto = if js::truthy(v) { js::string(v) } else { String::new() };
        if js::e_data_ymd(&texto) { texto } else { String::new() }
    };
    let legado_inicio = normalizar_data(body.get("periodoFechadasInicio"));
    let legado_fim = normalizar_data(body.get("periodoFechadasFim"));
    let recebido_periodo = body.get("periodoFechadasPorCasa").filter(|v| v.is_object());
    let mut periodos = Map::new();
    for casa in CASAS {
        let raw = recebido_periodo.and_then(|o| o.get(casa));
        // `raw?.inicio ?? legado.inicio`: só null/undefined usam o valor legado.
        let campo = |nome: &str, legado: &str| match raw.and_then(|r| r.get(nome)) {
            None | Some(Value::Null) => legado.to_string(),
            Some(v) => normalizar_data(Some(v)),
        };
        let inicio = campo("inicio", &legado_inicio);
        let fim = campo("fim", &legado_fim);
        if !inicio.is_empty() && !fim.is_empty() && inicio > fim {
            return Ok(erro(StatusCode::BAD_REQUEST, "A data inicial não pode ser posterior à data final."));
        }
        periodos.insert(casa.into(), json!({ "inicio": inicio, "fim": fim }));
    }

    let user_id = estado.usuario.id.clone();
    let regras_json = js::para_texto(&regras_globais);
    let por_casa_json = js::para_texto(&stake_por_casa);
    let periodos_json = js::para_texto(&periodos);
    estado
        .db
        .call(move |c| {
            c.execute(
                "INSERT INTO settings(user_id,stake_unidade,stake_regras,stake_por_casa,closed_range_start,closed_range_end,closed_range_by_platform,updated_at) \
                 VALUES(?,?,?,?,?,?,?,?) ON CONFLICT(user_id) DO UPDATE SET stake_unidade=excluded.stake_unidade,stake_regras=excluded.stake_regras,\
                 stake_por_casa=excluded.stake_por_casa,closed_range_start=excluded.closed_range_start,closed_range_end=excluded.closed_range_end,\
                 closed_range_by_platform=excluded.closed_range_by_platform,updated_at=excluded.updated_at",
                params![user_id, stake, regras_json, por_casa_json, "", "", periodos_json, db::agora()],
            )?;
            Ok(())
        })
        .await?;
    Ok(ok(json!({ "ok": true })))
}

/// `r && /^\d{4}-\d{2}-\d{2}$/.test(String(r.from)) && Number(r.unidade) > 0`.
fn filtrar_regras(valor: Option<&Value>) -> Vec<Value> {
    let Some(Value::Array(lista)) = valor else { return Vec::new() };
    lista
        .iter()
        .filter(|r| js::truthy(Some(r)))
        .filter_map(|r| {
            let from = js::string(r.get("from"));
            let unidade = js::number(r.get("unidade"));
            (js::e_data_ymd(&from) && unidade > 0.0)
                .then(|| json!({ "from": from, "unidade": js::numero_json(unidade) }))
        })
        .collect()
}

fn ler_json(corpo: &[u8], limite: usize) -> Result<Value, ApiResponse> {
    if corpo.len() > limite {
        return Err(erro(StatusCode::PAYLOAD_TOO_LARGE, "Requisição grande demais."));
    }
    if corpo.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_slice(corpo).map_err(|_| erro(StatusCode::BAD_REQUEST, "JSON inválido."))
}

fn ok(body: Value) -> ApiResponse {
    ApiResponse { status: StatusCode::OK, body, bruto: None }
}

fn erro(status: StatusCode, mensagem: &str) -> ApiResponse {
    ApiResponse { status, body: json!({ "ok": false, "error": mensagem }), bruto: None }
}

fn nao_migrada(method: &Method, path: &str) -> ApiResponse {
    ApiResponse {
        bruto: None,
        status: StatusCode::NOT_IMPLEMENTED,
        body: json!({
            "ok": false,
            "error": format!("Rota {method} {path} ainda não foi migrada para o backend Rust.")
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn estado() -> (tempfile::TempDir, Estado) {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = db::abrir(&dir.path().join("t.db")).unwrap();
        let usuario = db::usuario_local(&mut conn).unwrap();
        let chave = Arc::new(MasterKey::from_hex(&"11".repeat(32)).unwrap());
        (dir, Estado { db: Db::new(conn), chave, usuario, kalshi: Default::default(), poly: Default::default(), tracked: Default::default() })
    }

    fn chamar(e: &Estado, method: Method, path: &str, corpo: Value) -> ApiResponse {
        let bytes = if corpo.is_null() { Vec::new() } else { serde_json::to_vec(&corpo).unwrap() };
        tauri::async_runtime::block_on(handle(e, &method, path, None, &bytes))
    }

    #[test]
    fn me_sem_contas_e_com_wallet_mascarada() {
        let (_d, e) = estado();
        let r = chamar(&e, Method::GET, "/api/me", Value::Null);
        assert_eq!(r.status, StatusCode::OK);
        for casa in CASAS {
            assert_eq!(r.body["connections"][casa], false, "{casa}");
        }
        assert_eq!(r.body["user"]["name"], "Usuário Local");

        let wallet = "0xAbCd000000000000000000000000000000001234";
        let cifrada = e.chave.encrypt(wallet).unwrap();
        let id = e.usuario.id.clone();
        tauri::async_runtime::block_on(e.db.call(move |c| {
            c.execute("UPDATE connections SET polymarket_wallet_enc=? WHERE user_id=?", params![cifrada, id])?;
            Ok(())
        }))
        .unwrap();
        let r = chamar(&e, Method::GET, "/api/me", Value::Null);
        assert_eq!(r.body["connections"]["polymarket"], true);
        assert_eq!(r.body["connections"]["polymarketWalletHint"], "0xAbCd…1234");
        assert!(!r.body.to_string().contains(wallet), "a wallet completa nunca sai do backend");
    }

    #[test]
    fn settings_padrao_e_ida_e_volta() {
        let (_d, e) = estado();
        let r = chamar(&e, Method::GET, "/api/settings", Value::Null);
        assert_eq!(r.body["config"]["stakeUnidade"], 1);
        assert_eq!(r.body["config"]["stakeRegras"], json!([]));
        assert_eq!(r.body["config"]["stakePorCasa"], Value::Null);

        let r = chamar(
            &e,
            Method::POST,
            "/api/settings",
            json!({
                "stakeUnidade": "10",
                "stakeRegras": [{"from": "2026-01-01", "unidade": 20}, {"from": "ruim", "unidade": 5}, null, {"from": "2026-02-01", "unidade": 0}],
                "stakePorCasa": { "kalshi": { "unidade": 5, "regras": [] }, "sxbet": { "unidade": -1 } },
                "periodoFechadasPorCasa": { "polymarket": { "inicio": "2026-01-01", "fim": "2026-06-30" } },
                "periodoFechadasInicio": "2025-01-01"
            }),
        );
        assert_eq!(r.status, StatusCode::OK, "{}", r.body);

        let r = chamar(&e, Method::GET, "/api/settings", Value::Null);
        let c = &r.body["config"];
        assert_eq!(c["stakeUnidade"], 10);
        assert_eq!(c["stakeRegras"], json!([{"from": "2026-01-01", "unidade": 20}]));
        assert_eq!(c["stakePorCasa"]["kalshi"], json!({"unidade": 5, "regras": []}));
        assert_eq!(c["stakePorCasa"]["sxbet"], json!({"unidade": 10, "regras": [{"from": "2026-01-01", "unidade": 20}]}));
        assert_eq!(c["periodoFechadasPorCasa"]["polymarket"], json!({"inicio": "2026-01-01", "fim": "2026-06-30"}));
        assert_eq!(c["periodoFechadasPorCasa"]["kalshi"], json!({"inicio": "2025-01-01", "fim": ""}));
        assert_eq!(c["periodoFechadasInicio"], "");
    }

    #[test]
    fn settings_rejeita_valores_invalidos() {
        let (_d, e) = estado();
        let r = chamar(&e, Method::POST, "/api/settings", json!({ "stakeUnidade": 0 }));
        assert_eq!(r.status, StatusCode::BAD_REQUEST);
        let r = chamar(
            &e,
            Method::POST,
            "/api/settings",
            json!({ "stakeUnidade": 1, "periodoFechadasPorCasa": { "kalshi": { "inicio": "2026-05-01", "fim": "2026-01-01" } } }),
        );
        assert_eq!(r.status, StatusCode::BAD_REQUEST);
        assert_eq!(r.body["error"], "A data inicial não pode ser posterior à data final.");
        let r = tauri::async_runtime::block_on(handle(&e, &Method::POST, "/api/settings", None, b"{nao json"));
        assert_eq!(r.status, StatusCode::BAD_REQUEST);
    }

    /// Despeja as respostas de uma lista de URLs num banco real (cópia), para
    /// comparar com o `server.js`. Variáveis: BETTY_COMPARAR_DB,
    /// BETTY_COMPARAR_URLS e BETTY_COMPARAR_SAIDA.
    #[test]
    #[ignore]
    fn despejar_rotas_para_comparacao() {
        use std::io::Write;
        let var = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("defina {k}"));
        let mut conn = db::abrir(std::path::Path::new(&var("BETTY_COMPARAR_DB"))).unwrap();
        let usuario = db::usuario_local(&mut conn).unwrap();
        // Chave real (opcional) lida do local-config.json, para as rotas que decifram credenciais.
        let hex = std::env::var("BETTY_COMPARAR_CONFIG")
            .ok()
            .map(|p| {
                let v: Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
                v["masterKeyHex"].as_str().unwrap().to_string()
            })
            .unwrap_or_else(|| "00".repeat(32));
        let chave = Arc::new(MasterKey::from_hex(&hex).unwrap());
        let e = Estado { db: Db::new(conn), chave, usuario, kalshi: Default::default(), poly: Default::default(), tracked: Default::default() };
        let arquivo_urls = var("BETTY_COMPARAR_URLS");
        let pasta = std::path::Path::new(&arquivo_urls).parent().unwrap().to_path_buf();
        let urls = std::fs::read_to_string(&arquivo_urls).unwrap();
        let mut saida = std::fs::File::create(var("BETTY_COMPARAR_SAIDA")).unwrap();
        for url in urls.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            let partes: Vec<&str> = url.split_whitespace().collect();
            let (metodo, rota, corpo) = if url.starts_with('/') {
                (Method::GET, url, Vec::new())
            } else {
                let corpo = partes.get(2).map(|f| std::fs::read(pasta.join(f)).unwrap()).unwrap_or_default();
                (Method::from_bytes(partes[0].as_bytes()).unwrap(), partes[1], corpo)
            };
            let (path, query) = rota.split_once('?').map_or((rota, None), |(p, q)| (p, Some(q)));
            let inicio = std::time::Instant::now();
            let r = tauri::async_runtime::block_on(handle(&e, &metodo, path, query, &corpo));
            // Inclui a serialização, como o `res.json()` do Node.
            let bytes = match r.bruto { Some(b) => b, None => js::para_vec(&r.body) };
            let ms = inicio.elapsed().as_secs_f64() * 1000.0;
            let body: Value = serde_json::from_slice(&bytes).unwrap();
            let linha = json!({ "url": url, "status": r.status.as_u16(), "ms": ms, "body": body });
            writeln!(saida, "{linha}").unwrap();
        }
    }

    /// Perfil do `delta` por fase, com os lotes de `BETTY_COMPARAR_URLS`
    /// sobre `BETTY_COMPARAR_DB` (5 voltas; a partir da 2ª tudo é inalterado).
    #[test]
    #[ignore]
    fn medir_delta() {
        use std::time::Instant;
        let var = |k: &str| std::env::var(k).unwrap();
        let caminho = var("BETTY_COMPARAR_URLS");
        let pasta = std::path::Path::new(&caminho).parent().unwrap().to_path_buf();
        let corpos: Vec<Vec<u8>> = std::fs::read_to_string(&caminho)
            .unwrap()
            .lines()
            .filter_map(|l| l.split_whitespace().nth(2))
            .map(|f| std::fs::read(pasta.join(f)).unwrap())
            .collect();
        let mut conn = db::abrir(std::path::Path::new(&var("BETTY_COMPARAR_DB"))).unwrap();
        let usuario = db::usuario_local(&mut conn).unwrap();
        let user = usuario.id.clone();
        for volta in 1..=5 {
            let (mut t_ler, mut t_gravar, mut t_prep, mut t_ser) = (0.0, 0.0, 0.0, 0.0);
            for corpo in &corpos {
                let i = Instant::now();
                let body: Value = serde_json::from_slice(corpo).unwrap();
                t_ler += i.elapsed().as_secs_f64();
                let linhas = body["fechados"].as_array().cloned().unwrap();
                let i = Instant::now();
                let g = ledger::gravar(&mut conn, &user, linhas, true).unwrap();
                t_gravar += i.elapsed().as_secs_f64();
                let i = Instant::now();
                let fechados = ledger::preparar_titulos_kalshi(&conn, &user, g.processadas, body.get("titleVersion")).unwrap();
                t_prep += i.elapsed().as_secs_f64();
                let i = Instant::now();
                let bytes = js::para_vec(&json!({ "ok": true, "fechados": fechados }));
                t_ser += i.elapsed().as_secs_f64();
                assert!(!bytes.is_empty());
            }
            let ms = |s: f64| s * 1000.0;
            eprintln!(
                "volta {volta}: total {:.0}ms | ler corpo {:.0} | gravar {:.0} | títulos {:.0} | serializar {:.0}",
                ms(t_ler + t_gravar + t_prep + t_ser),
                ms(t_ler),
                ms(t_gravar),
                ms(t_prep),
                ms(t_ser)
            );
        }
    }

    #[test]
    fn versoes_do_indice_ficam_no_banco() {
        let (_d, e) = estado();
        let status = chamar(&e, Method::GET, "/api/history/closed/index/status", Value::Null);
        assert_eq!(status.body["versions"], json!({}), "banco novo sem versões");
        let r = chamar(
            &e,
            Method::POST,
            "/api/history/closed/index/versions",
            json!({ "versions": { "category": "7", "title": " ", "date": 3, "outra": "x" } }),
        );
        assert_eq!(r.status, StatusCode::OK);
        assert_eq!(r.body["versions"], json!({ "category": "7" }), "vazio, número e tipo desconhecido são ignorados");
        chamar(&e, Method::POST, "/api/history/closed/index/versions", json!({ "versions": { "category": "8", "date": "3" } }));
        let status = chamar(&e, Method::GET, "/api/history/closed/index/status", Value::Null);
        assert_eq!(status.body["versions"], json!({ "category": "8", "date": "3" }));
        assert_eq!(status.body["complete"], json!(true));
    }

    #[test]
    fn rota_desconhecida_responde_501() {
        let (_d, e) = estado();
        let r = chamar(&e, Method::GET, "/api/users", Value::Null);
        assert_eq!(r.status, StatusCode::NOT_IMPLEMENTED);
    }

    #[test]
    fn casas_rastreadas_sem_conexao() {
        let (_d, e) = estado();
        let r = chamar(&e, Method::POST, "/api/tracked/predict/sync", json!({}));
        assert_eq!((r.status, r.body["error"].as_str()), (StatusCode::CONFLICT, Some("Conta não conectada.")));
        let r = chamar(&e, Method::GET, "/api/tracked/sxbet/balance", Value::Null);
        assert_eq!(r.status, StatusCode::CONFLICT);
        let r = chamar(&e, Method::GET, "/api/connections/predict/test", Value::Null);
        assert_eq!(r.body["error"].as_str(), Some("Conecte a casa primeiro."));
        let r = chamar(&e, Method::POST, "/api/connections/outra", json!({ "apiKey": "x" }));
        assert_eq!(r.status, StatusCode::NOT_FOUND);
        let r = chamar(&e, Method::POST, "/api/connections/predict", json!({ "apiKey": "abc", "wallet": "nao" }));
        assert_eq!(r.status, StatusCode::BAD_REQUEST);
        let r = chamar(&e, Method::POST, "/api/connections/sxbet", json!({ "apiKey": "com espaço" }));
        assert_eq!(r.body["error"].as_str(), Some("Informe uma chave de API válida."));
        let r = chamar(&e, Method::DELETE, "/api/connections/sxbet", Value::Null);
        assert_eq!(r.status, StatusCode::OK);
        assert_eq!(r.body["connections"]["sxbet"], json!(false));
    }
}
