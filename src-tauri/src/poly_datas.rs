//! Recuperação da data oficial de resolução de perdas da Polymarket ainda não
//! resgatadas, igual a `lib/pnl-date-recovery.js` + `recuperarResolucaoPoly()`.
//! A fase de rede só lê a Gamma API; o condition ID precisa bater exatamente.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

use crate::audit;
use crate::db::{Db, agora};
use crate::history::ymd_sao_paulo;
use crate::{js, net};

/// Última tentativa por `usuario|chave` (no máximo uma a cada 10 minutos).
static TENTATIVAS: LazyLock<Mutex<HashMap<String, f64>>> = LazyLock::new(Default::default);
const INTERVALO_MS: f64 = 10.0 * 60.0 * 1000.0;

pub struct Recuperacao {
    pub salvas: usize,
    pub pendentes: usize,
    pub datas: HashMap<String, String>,
}

struct Resultado {
    chave: String,
    condicao: String,
    data: Option<String>,
    erro: bool,
}

async fn get_com_repeticao(url: &str) -> Result<Value> {
    let mut ultimo = anyhow!("sem tentativa");
    for _ in 0..3 {
        match net::CLIENTE.get(url).timeout(Duration::from_secs(15)).send().await {
            Ok(r) if r.status().is_success() => return Ok(serde_json::from_str(&r.text().await?)?),
            Ok(r) => ultimo = anyhow!("HTTP {}", r.status().as_u16()),
            Err(e) => ultimo = e.into(),
        }
    }
    Err(ultimo)
}

fn texto(v: Option<&Value>) -> String {
    if js::truthy(v) { js::string(v) } else { String::new() }
}

async fn buscar_uma(chave: String, aposta: Value) -> Resultado {
    let condicao = texto(crate::titles::ou(aposta.get("conditionId"), aposta.get("condition_id"))).to_lowercase();
    let slug = aposta.get("slug").cloned();
    let resultado: Result<String> = async {
        if condicao.is_empty() {
            bail!("Missing exact condition ID");
        }
        let lista = |d: &Value| d.as_array().cloned().unwrap_or_default();
        let mesma_condicao = |m: &Value| texto(m.get("conditionId")).to_lowercase() == condicao;
        let mut mercado = None;
        if js::truthy(slug.as_ref()) {
            let s = js::string(slug.as_ref());
            let dados = get_com_repeticao(&format!("https://gamma-api.polymarket.com/markets?closed=true&slug={}", net::codificar(&s))).await?;
            mercado = lista(&dados).into_iter().find(|m| mesma_condicao(m) && m.get("slug") == slug.as_ref());
        }
        if mercado.is_none() {
            let url = format!("https://gamma-api.polymarket.com/markets?closed=true&condition_ids={}", net::codificar(&condicao));
            mercado = lista(&get_com_repeticao(&url).await?).into_iter().find(|m| mesma_condicao(m));
        }
        let ms = mercado.as_ref().and_then(|m| audit::timestamp_ms(m.get("closedTime"))).filter(|v| *v != 0.0);
        match (mercado.as_ref().and_then(|m| m.get("closed")), ms) {
            (Some(Value::Bool(true)), Some(ms)) => js::iso(ms).ok_or_else(|| anyhow!("data inválida")),
            _ => bail!("No confirmed market closure date"),
        }
    }
    .await;
    match resultado {
        Ok(data) => Resultado { chave, condicao, data: Some(data), erro: false },
        Err(_) => Resultado { chave, condicao, data: None, erro: true },
    }
}

/// `recuperarResolucaoPoly()`. `entradas` = `(chave, aposta)`; `None` = todas
/// as perdas resolvidas do ledger ainda sem data.
pub async fn recuperar(db: &Db, user_id: &str, entradas: Option<Vec<(String, Value)>>) -> Result<Recuperacao> {
    let linhas = match entradas {
        Some(e) => e,
        None => {
            let u = user_id.to_string();
            db.ler(move |c| {
                let mut s = c.prepare(
                    "SELECT bet_key,data_json FROM closed_bets_ledger WHERE user_id=? AND platform='polymarket'
    AND json_extract(data_json,'$.redeemable')=1 AND json_extract(data_json,'$.curPrice')=0
    AND COALESCE(json_extract(data_json,'$._resolvedTime'),'')=''",
                )?;
                let rows = s.query_map([&u], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
                let mut v = Vec::new();
                for r in rows {
                    let (k, d) = r?;
                    v.push((k, serde_json::from_str(&d)?));
                }
                Ok(v)
            })
            .await?
        }
    };
    let agora_ms = chrono::Utc::now().timestamp_millis() as f64;
    let pendentes: Vec<(String, Value)> = {
        let mut tentativas = TENTATIVAS.lock().map_err(|_| anyhow!("estado indisponível"))?;
        linhas
            .into_iter()
            .filter(|(chave, aposta)| {
                if !audit::resolved_loss(aposta) || audit::timestamp_ms(aposta.get("_resolvedTime")).is_some_and(|v| v != 0.0) {
                    return false;
                }
                let k = format!("{user_id}|{chave}");
                if agora_ms - tentativas.get(&k).copied().unwrap_or(0.0) < INTERVALO_MS {
                    return false;
                }
                tentativas.insert(k, agora_ms);
                true
            })
            .collect()
    };
    let mut resultados = Vec::new();
    for lote in pendentes.chunks(6) {
        resultados.extend(futures::future::join_all(lote.iter().cloned().map(|(k, a)| buscar_uma(k, a))).await);
    }
    let pendentes_erro = resultados.iter().filter(|r| r.erro).count();
    let datas: HashMap<String, String> = resultados.iter().filter_map(|r| r.data.clone().map(|d| (r.chave.clone(), d))).collect();
    let u = user_id.to_string();
    let para_gravar: Vec<(String, String, String)> =
        resultados.into_iter().filter_map(|r| r.data.map(|d| (r.chave, r.condicao, d))).collect();
    let salvas = if para_gravar.is_empty() { 0 } else { db.call(move |c| aplicar(c, &u, para_gravar)).await? };
    Ok(Recuperacao { salvas, pendentes: pendentes_erro, datas })
}

/// `applyResolutionDates()`: relê dentro da transação e nunca sobrescreve uma
/// data já confirmada nem uma aposta de outro condition ID.
fn aplicar(c: &mut rusqlite::Connection, user_id: &str, resultados: Vec<(String, String, String)>) -> Result<usize> {
    let tx = c.transaction()?;
    let mut salvas = 0;
    {
        let mut ler = tx.prepare("SELECT data_json FROM closed_bets_ledger WHERE user_id=? AND platform='polymarket' AND bet_key=?")?;
        let mut gravar = tx.prepare("UPDATE closed_bets_ledger SET data_json=?,updated_at=? WHERE user_id=? AND platform='polymarket' AND bet_key=?")?;
        let mut indice = tx.prepare("UPDATE closed_bets_index SET pnl_ts=?,pnl_ymd=? WHERE user_id=? AND platform='polymarket' AND bet_key=?")?;
        for (chave, condicao, data) in resultados {
            if audit::timestamp_ms(Some(&json!(data))).is_none() {
                continue;
            }
            let Some(atual) = ler.query_row(params![user_id, chave], |r| r.get::<_, String>(0)).optional()? else { continue };
            let d: Value = serde_json::from_str(&atual)?;
            if audit::timestamp_ms(d.get("_resolvedTime")).is_some_and(|v| v != 0.0) {
                continue;
            }
            if texto(crate::titles::ou(d.get("conditionId"), d.get("condition_id"))).to_lowercase() != condicao.to_lowercase() {
                continue;
            }
            let mut nova = d.as_object().cloned().unwrap_or_default();
            nova.insert("_resolvedTime".into(), json!(data));
            nova.insert("_resolutionDateSource".into(), json!("gamma-market-closedTime"));
            let protegida = audit::protect_closed_date(Some(&d), Value::Object(nova));
            gravar.execute(params![js::para_texto(&protegida), agora(), user_id, chave])?;
            salvas += 1;
            let ts = audit::pnl_timestamp(&protegida, None);
            indice.execute(params![ts, if ts != 0.0 { ymd_sao_paulo(ts) } else { String::new() }, user_id, chave])?;
        }
    }
    tx.commit()?;
    Ok(salvas)
}
