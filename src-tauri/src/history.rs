//! Leitura do histórico fechado (`/api/history/closed/*`), com as mesmas
//! consultas e o mesmo contrato do `server.js`. O ledger (`closed_bets_ledger`)
//! é a fonte canônica; o índice (`closed_bets_index`) serve filtros, paginação
//! e agregações.

use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::{Context, Result};
use chrono::{Datelike, NaiveDate, TimeZone, Utc};
use chrono_tz::America::Sao_Paulo;
use indexmap::IndexMap;
use rusqlite::types::{ToSql, Value as SqlValue, ValueRef};
use rusqlite::{Connection, OptionalExtension, params_from_iter};
use serde::Serialize;
use serde_json::value::RawValue;
use serde_json::{Map, Value, json};

use crate::api::CASAS;
use crate::db::agora;
use crate::js;
use crate::odds;

/// Parâmetros da URL, com a semântica de `req.query` do Express.
pub type Consulta = HashMap<String, String>;

pub fn ler_consulta(query: Option<&str>) -> Consulta {
    let mut mapa: Consulta = HashMap::new();
    for (k, v) in url::form_urlencoded::parse(query.unwrap_or("").as_bytes()) {
        // Chave repetida vira array no Express; `String(array)` junta com vírgula.
        mapa.entry(k.into_owned()).and_modify(|a| *a = format!("{a},{v}")).or_insert_with(|| v.into_owned());
    }
    mapa
}

/// `raw.a || raw.b || padrão`: string vazia conta como ausente.
fn primeiro<'a>(c: &'a Consulta, chaves: &[&str], padrao: &'a str) -> &'a str {
    chaves.iter().filter_map(|k| c.get(*k)).find(|v| !v.is_empty()).map_or(padrao, |s| s.as_str())
}

fn numero(c: &Consulta, chave: &str) -> f64 {
    c.get(chave).map_or(f64::NAN, |v| js::number(Some(&Value::String(v.clone()))))
}

/// `Number(x) || padrão`.
fn numero_ou(c: &Consulta, chave: &str, padrao: f64) -> f64 {
    let n = numero(c, chave);
    if n == 0.0 || n.is_nan() { padrao } else { n }
}

// ---------------------------------------------------------------------------
// Datas
// ---------------------------------------------------------------------------

/// Dia em São Paulo para um timestamp em ms, independente do fuso do computador.
pub fn ymd_sao_paulo(ms: f64) -> String {
    if !ms.is_finite() || ms == 0.0 {
        return String::new();
    }
    match Utc.timestamp_millis_opt(ms.trunc() as i64).single() {
        Some(t) => t.with_timezone(&Sao_Paulo).format("%Y-%m-%d").to_string(),
        None => String::new(),
    }
}

fn hoje_ms() -> f64 {
    Utc::now().timestamp_millis() as f64
}

fn data_ymd(ymd: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(ymd, "%Y-%m-%d").ok()
}

fn somar_dias(ymd: &str, dias: i64) -> String {
    data_ymd(ymd)
        .and_then(|d| d.checked_add_signed(chrono::TimeDelta::days(dias)))
        .map_or_else(String::new, |d| d.format("%Y-%m-%d").to_string())
}

fn diferenca_dias(a: &str, b: &str) -> i64 {
    match (data_ymd(a), data_ymd(b)) {
        (Some(a), Some(b)) => (b - a).num_days(),
        _ => 0,
    }
}

fn semana_iso(ymd: &str) -> String {
    data_ymd(ymd).map_or_else(String::new, |d| {
        let w = d.iso_week();
        format!("{}-W{:02}", w.year(), w.week())
    })
}

fn utc_dia_ms(ymd: &str) -> Option<i64> {
    data_ymd(ymd).map(|d| d.and_hms_opt(0, 0, 0).expect("meia-noite").and_utc().timestamp_millis())
}

// ---------------------------------------------------------------------------
// Configuração do usuário
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct StakeCasa {
    unidade: f64,
    /// (from, unidade), em ordem crescente de data.
    regras: Vec<(String, f64)>,
}

fn json_ou(texto: Option<String>, padrao: &str) -> Value {
    let t = texto.filter(|t| !t.is_empty()).unwrap_or_else(|| padrao.to_string());
    serde_json::from_str(&t).unwrap_or(Value::Null)
}

/// `configStakeHistorico()`.
fn config_stake(c: &Connection, user_id: &str) -> Result<HashMap<&'static str, StakeCasa>> {
    let linha: Option<(Option<f64>, Option<String>, Option<String>)> = c
        .query_row(
            "SELECT stake_unidade,stake_regras,stake_por_casa FROM settings WHERE user_id=?",
            [user_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let (unidade, regras, por_casa) = linha.unwrap_or((None, None, None));
    let por_casa = json_ou(por_casa, "{}");
    let legadas = json_ou(regras, "[]");
    let base = unidade.filter(|u| *u > 0.0).unwrap_or(1.0);
    let normalizar_regras = |lista: &Value| -> Vec<(String, f64)> {
        let mut v: Vec<(String, f64)> = lista
            .as_array()
            .map(|a| a.as_slice())
            .unwrap_or_default()
            .iter()
            .filter_map(|r| {
                let from = if js::truthy(r.get("from")) { js::string(r.get("from")) } else { String::new() };
                let u = js::number(r.get("unidade"));
                (js::e_data_ymd(&from) && u > 0.0).then_some((from, u))
            })
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    };
    let mut config = HashMap::new();
    for casa in CASAS {
        let raw = por_casa.get(casa);
        let u = js::number(raw.and_then(|r| r.get("unidade")));
        let regras = match raw.and_then(|r| r.get("regras")) {
            Some(lista @ Value::Array(_)) => normalizar_regras(lista),
            _ => normalizar_regras(&legadas),
        };
        config.insert(casa, StakeCasa { unidade: if u > 0.0 { u } else { base }, regras });
    }
    Ok(config)
}

/// `unidadeStakeSql()`: unidade de stake vigente na data da tabela, por casa.
fn unidade_stake_sql(config: &HashMap<&'static str, StakeCasa>) -> (String, Vec<SqlValue>) {
    let mut params = Vec::new();
    let mut partes = Vec::new();
    for casa in CASAS {
        let c = config.get(casa).cloned().unwrap_or(StakeCasa { unidade: 1.0, regras: vec![] });
        let mut regras = c.regras.clone();
        regras.sort_by(|a, b| b.0.cmp(&a.0));
        let mut whens = Vec::new();
        for (from, u) in regras {
            params.push(SqlValue::Text(from));
            params.push(SqlValue::Real(u));
            whens.push("WHEN i.table_ymd>=? THEN ?");
        }
        params.push(SqlValue::Real(if c.unidade > 0.0 { c.unidade } else { 1.0 }));
        let sql = if whens.is_empty() { "?".to_string() } else { format!("(CASE {} ELSE ? END)", whens.join(" ")) };
        partes.push(format!("WHEN '{casa}' THEN {sql}"));
    }
    (format!("(CASE i.platform {} ELSE 1 END)", partes.join(" ")), params)
}

/// `periodoFechadasConfiguradoSql()`: recorte de datas configurado por casa.
fn periodos_configurados(c: &Connection, user_id: &str) -> Result<Vec<(&'static str, String, String)>> {
    let linha: Option<(Option<String>, Option<String>, Option<String>)> = c
        .query_row(
            "SELECT closed_range_start,closed_range_end,closed_range_by_platform FROM settings WHERE user_id=?",
            [user_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let (inicio, fim, por_casa) = linha.unwrap_or((None, None, None));
    let normalizar = |v: Option<&Value>| -> String {
        let s = if js::truthy(v) { js::string(v) } else { String::new() };
        if js::e_data_ymd(&s) { s } else { String::new() }
    };
    let legado_inicio = normalizar(inicio.map(Value::String).as_ref());
    let legado_fim = normalizar(fim.map(Value::String).as_ref());
    let por_casa = json_ou(por_casa, "{}");
    let possui = CASAS.iter().any(|c| por_casa.get(*c).is_some_and(|v| v.is_object() || v.is_array()));
    Ok(CASAS
        .iter()
        .map(|casa| {
            let raw = por_casa.get(*casa);
            if possui {
                (*casa, normalizar(raw.and_then(|r| r.get("inicio"))), normalizar(raw.and_then(|r| r.get("fim"))))
            } else {
                (*casa, legado_inicio.clone(), legado_fim.clone())
            }
        })
        .collect())
}

fn conta_ativa_sql(alias: &str) -> String {
    format!(
        "({alias}.platform IN ('polymarket', 'kalshi') OR EXISTS (\n    SELECT 1 FROM tracked_connections c WHERE c.user_id={alias}.user_id AND c.platform={alias}.platform\n    AND substr({alias}.bet_key, 1, 65)=c.account_scope || ':'))"
    )
}

struct Filtro {
    onde: Vec<String>,
    params: Vec<SqlValue>,
}

impl Filtro {
    fn sql(&self) -> String {
        self.onde.join(" AND ")
    }
}

fn aplicar_periodo_configurado(c: &Connection, user_id: &str, f: &mut Filtro) -> Result<()> {
    let periodos = periodos_configurados(c, user_id)?;
    if !periodos.iter().any(|(_, i, f)| !i.is_empty() || !f.is_empty()) {
        return Ok(());
    }
    let mut clausulas = Vec::new();
    for (casa, inicio, fim) in periodos {
        let mut partes = vec!["i.platform=?".to_string()];
        f.params.push(SqlValue::Text(casa.into()));
        if !inicio.is_empty() {
            partes.push("i.pnl_ymd<>'' AND i.pnl_ymd>=?".into());
            f.params.push(SqlValue::Text(inicio));
        }
        if !fim.is_empty() {
            partes.push("i.pnl_ymd<>'' AND i.pnl_ymd<=?".into());
            f.params.push(SqlValue::Text(fim));
        }
        clausulas.push(format!("({})", partes.join(" AND ")));
    }
    f.onde.push(format!("({})", clausulas.join(" OR ")));
    Ok(())
}

/// `filtrosHistoricoSql()`.
fn filtros(c: &Connection, user_id: &str, raw: &Consulta) -> Result<Filtro> {
    let mut f = Filtro { onde: vec!["i.user_id=?".into(), conta_ativa_sql("i")], params: vec![SqlValue::Text(user_id.into())] };
    aplicar_periodo_configurado(c, user_id, &mut f)?;
    let casa = primeiro(raw, &["platform", "site"], "todas").to_lowercase();
    if CASAS.contains(&casa.as_str()) {
        f.onde.push("i.platform=?".into());
        f.params.push(SqlValue::Text(casa));
    }
    let categoria = primeiro(raw, &["category"], "todos");
    if !categoria.is_empty() && categoria != "todos" {
        f.onde.push("i.category=?".into());
        f.params.push(SqlValue::Text(categoria.into()));
    }
    let faixa_id = raw.get("oddsRange").map_or("undefined", |s| s.as_str());
    if let Some(faixa) = odds::faixa(faixa_id) {
        f.onde.push("bet_round_odd(i.odd_value)>=?".into());
        f.params.push(SqlValue::Real(faixa.min));
        if let Some(max) = faixa.max {
            f.onde.push("bet_round_odd(i.odd_value)<=?".into());
            f.params.push(SqlValue::Real(max));
        }
    }
    let realizacao = raw.get("dateBasis").is_some_and(|d| d == "realization");
    let coluna = if realizacao { "i.pnl_ymd" } else { "i.start_ymd" };
    let inicio = primeiro(raw, &["start", "startYmd"], "");
    let fim = primeiro(raw, &["end", "endYmd"], "");
    if js::e_data_ymd(inicio) {
        f.onde.push(format!("{coluna}<>'' AND {coluna}>=?"));
        f.params.push(SqlValue::Text(inicio.into()));
    }
    if js::e_data_ymd(fim) {
        f.onde.push(format!("{coluna}<>'' AND {coluna}<=?"));
        f.params.push(SqlValue::Text(fim.into()));
    }
    let dia = raw.get("day").map_or("", |s| s.as_str());
    if js::e_data_ymd(dia) {
        f.onde.push(if realizacao { "i.pnl_ymd=?" } else { "i.table_ymd=?" }.into());
        f.params.push(SqlValue::Text(dia.into()));
    }
    let titulo = raw.get("title").map_or("", |s| s.as_str()).trim().to_lowercase();
    if !titulo.is_empty() {
        // `titleAlt`: o mesmo termo com as palavras do índice (em português),
        // enviado pela interface em outro idioma.
        let alternativo = raw.get("titleAlt").map_or("", |s| s.as_str()).trim().to_lowercase();
        if alternativo.is_empty() {
            f.onde.push("i.title_search LIKE ? ESCAPE '\\'".into());
            f.params.push(padrao_like(&titulo));
        } else {
            f.onde.push("(i.title_search LIKE ? ESCAPE '\\' OR i.title_search LIKE ? ESCAPE '\\')".into());
            f.params.push(padrao_like(&titulo));
            f.params.push(padrao_like(&alternativo));
        }
    }
    Ok(f)
}

/// "Contém" no `LIKE`, com `\`, `%` e `_` do termo tratados como texto.
fn padrao_like(termo: &str) -> SqlValue {
    let mut escapado = String::with_capacity(termo.len() + 2);
    for ch in termo.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            escapado.push('\\');
        }
        escapado.push(ch);
    }
    SqlValue::Text(format!("%{escapado}%"))
}

// ---------------------------------------------------------------------------
// Métricas
// ---------------------------------------------------------------------------

/// `stakeHistorico()`.
pub(crate) fn stake_historico(aposta: &Value) -> f64 {
    let n = |k: &str| js::number(aposta.get(k));
    let inicial = n("initialValue");
    if inicial.is_finite() && inicial > 0.0 {
        return inicial;
    }
    let (comprado, media, tamanho) = (n("totalBought"), n("avgPrice"), n("size"));
    if comprado.is_finite() && media.is_finite() && media > 0.0 {
        return comprado * media;
    }
    if tamanho.is_finite() && media.is_finite() {
        return tamanho * media;
    }
    0.0
}

pub(crate) fn odd_historica(aposta: &Value) -> f64 {
    let preco = js::number(aposta.get("avgPrice"));
    if preco.is_finite() && preco > 0.0 { 1.0 / preco } else { 0.0 }
}

/// Há linhas do índice sem métricas? Só bancos vindos de versões antigas têm:
/// toda gravação do índice já sai com elas.
pub fn metricas_pendentes(c: &Connection, user_id: &str) -> Result<bool> {
    let pendentes: i64 =
        c.query_row("SELECT COUNT(*) FROM closed_bets_index WHERE user_id=? AND metrics_ready=0", [user_id], |r| r.get(0))?;
    Ok(pendentes > 0)
}

/// `garantirMetricasIndiceHistorico()`: completa métricas de linhas do índice
/// que ainda não as têm. Normalmente não há nada a fazer.
pub fn garantir_metricas(c: &mut Connection, user_id: &str) -> Result<()> {
    // As conexões de leitura e a do estado salvo não gravam: as métricas são
    // completadas antes, na conexão de escrita (`api::completar_metricas` e
    // `api::abrir_instantaneo`).
    if c.is_readonly(rusqlite::MAIN_DB)? || !metricas_pendentes(c, user_id)? {
        return Ok(());
    }
    let linhas: Vec<(String, String, String, f64)> = {
        let mut stmt = c.prepare(
            "SELECT l.platform,l.bet_key,l.data_json,i.pnl_ts FROM closed_bets_ledger l
             JOIN closed_bets_index i ON i.user_id=l.user_id AND i.platform=l.platform AND i.bet_key=l.bet_key
             WHERE l.user_id=? AND i.metrics_ready=0",
        )?;
        let rows = stmt.query_map([user_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, f64>(3)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let tx = c.transaction()?;
    {
        let mut update = tx.prepare(
            "UPDATE closed_bets_index SET pnl_ymd=?,pnl_value=?,stake_value=?,odd_value=?,won=?,metrics_ready=1,updated_at=?
             WHERE user_id=? AND platform=? AND bet_key=?",
        )?;
        for (platform, bet_key, data, pnl_ts) in linhas {
            let Ok(aposta) = serde_json::from_str::<Value>(&data) else { continue };
            let pnl = js::number(Some(aposta.get("realizedPnl").filter(|v| js::truthy(Some(v))).unwrap_or(&json!(0))));
            let pnl = if pnl.is_nan() { 0.0 } else { pnl };
            update.execute(rusqlite::params![
                ymd_sao_paulo(pnl_ts),
                pnl,
                stake_historico(&aposta),
                odd_historica(&aposta),
                i64::from(pnl > 0.0),
                agora(),
                user_id,
                platform,
                bet_key
            ])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Valor de uma coluna como o `better-sqlite3` entregaria ao JavaScript.
fn valor_sql(v: ValueRef<'_>) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => json!(i),
        ValueRef::Real(f) => js::numero_json(f),
        ValueRef::Text(t) => Value::String(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(_) => Value::Null,
    }
}

fn como_f64(v: Option<&Value>) -> f64 {
    let n = js::number(v);
    if n.is_nan() { 0.0 } else { n }
}

/// Monta o objeto de métricas como `normalizar()` do `metricasSql()`.
fn normalizar_metricas(mut linha: Map<String, Value>) -> Value {
    let g = |l: &Map<String, Value>, k: &str| como_f64(l.get(k));
    let total = g(&linha, "total");
    let lucro = g(&linha, "lucroD");
    let volume = g(&linha, "volumeD");
    let odd_count = g(&linha, "oddCount");
    let wins = g(&linha, "wins");
    let losses = g(&linha, "losses");
    let ganho = g(&linha, "ganhoBruto");
    let perda = g(&linha, "perdaBruta");
    let lucro_stake = g(&linha, "lucroStake");
    let volume_stake = g(&linha, "volumeStake");
    let pnl_hoje = g(&linha, "pnlHoje");
    let odd_soma = g(&linha, "oddSoma");
    let perda_abs = perda.abs();
    let n = js::numero_json;
    for (k, v) in [
        ("total", total),
        ("lucroD", lucro),
        ("volumeD", volume),
        ("lucroStake", lucro_stake),
        ("volumeStake", volume_stake),
        ("wins", wins),
        ("losses", losses),
        ("ganhoBruto", ganho),
        ("perdaBruta", perda),
        ("roi", if volume > 0.0 { lucro / volume * 100.0 } else { 0.0 }),
        ("taxaAcerto", if total != 0.0 { wins / total * 100.0 } else { 0.0 }),
        ("oddMedia", if odd_count != 0.0 { odd_soma / odd_count } else { 0.0 }),
        ("pnlHoje", pnl_hoje),
        ("avgPnl", if total != 0.0 { lucro / total } else { 0.0 }),
        ("avgPnlStake", if total != 0.0 { lucro_stake / total } else { 0.0 }),
        ("avgWin", if wins != 0.0 { ganho / wins } else { 0.0 }),
        ("avgLoss", if losses != 0.0 { perda / losses } else { 0.0 }),
    ] {
        linha.insert(k.into(), n(v));
    }
    let fator = if perda_abs > 0.0 {
        n(ganho / perda_abs)
    } else if ganho > 0.0 {
        Value::Null
    } else {
        json!(0)
    };
    linha.insert("profitFactor".into(), fator);
    Value::Object(linha)
}

fn consultar_objetos(c: &Connection, sql: &str, params: &[SqlValue]) -> Result<Vec<Map<String, Value>>> {
    let mut stmt = c.prepare(sql).with_context(|| format!("SQL inválido: {sql}"))?;
    let nomes: Vec<String> = stmt.column_names().map(|s| s.to_string()).collect();
    let mut rows = stmt.query(params_from_iter(params.iter()))?;
    let mut saida = Vec::new();
    while let Some(r) = rows.next()? {
        let mut m = Map::new();
        for (i, nome) in nomes.iter().enumerate() {
            m.insert(nome.clone(), valor_sql(r.get_ref(i)?));
        }
        saida.push(m);
    }
    Ok(saida)
}

/// `metricasSql()`: totais (ou grupos) do histórico filtrado.
fn metricas(c: &mut Connection, user_id: &str, raw: &Consulta, grupo: Option<&str>) -> Result<Value> {
    garantir_metricas(c, user_id)?;
    let f = filtros(c, user_id, raw)?;
    let (u, up) = unidade_stake_sql(&config_stake(c, user_id)?);
    let hoje = ymd_sao_paulo(hoje_ms());
    let select = format!(
        "COUNT(*) AS total,COALESCE(SUM(i.pnl_value),0) AS lucroD,COALESCE(SUM(i.stake_value),0) AS volumeD,
    COALESCE(SUM(i.pnl_value/{u}),0) AS lucroStake,COALESCE(SUM(i.stake_value/{u}),0) AS volumeStake,
    COALESCE(SUM(i.won),0) AS wins,COALESCE(SUM(CASE WHEN i.odd_value>0 THEN i.odd_value ELSE 0 END),0) AS oddSoma,
    COALESCE(SUM(CASE WHEN i.odd_value>0 THEN 1 ELSE 0 END),0) AS oddCount,
    COALESCE(SUM(CASE WHEN i.pnl_ymd=? THEN i.pnl_value ELSE 0 END),0) AS pnlHoje,
    COALESCE(SUM(CASE WHEN i.pnl_value>0 THEN i.pnl_value ELSE 0 END),0) AS ganhoBruto,
    COALESCE(SUM(CASE WHEN i.pnl_value<0 THEN i.pnl_value ELSE 0 END),0) AS perdaBruta,
    COALESCE(SUM(CASE WHEN i.pnl_value>0 THEN i.pnl_value/{u} ELSE 0 END),0) AS ganhoStake,
    COALESCE(SUM(CASE WHEN i.pnl_value<0 THEN i.pnl_value/{u} ELSE 0 END),0) AS perdaStake,
    COALESCE(SUM(CASE WHEN i.pnl_value<0 THEN 1 ELSE 0 END),0) AS losses"
    );
    let prefixo = grupo.map_or(String::new(), |g| format!("{g} AS chave,"));
    let sql = format!(
        "SELECT {prefixo}{select} FROM closed_bets_index i WHERE {}{}",
        f.sql(),
        grupo.map_or(String::new(), |g| format!(" GROUP BY {g}"))
    );
    let mut params = Vec::new();
    params.extend(up.iter().cloned());
    params.extend(up.iter().cloned());
    params.push(SqlValue::Text(hoje));
    params.extend(up.iter().cloned());
    params.extend(up.iter().cloned());
    params.extend(f.params);
    let linhas = consultar_objetos(c, &sql, &params)?;
    Ok(match grupo {
        Some(_) => Value::Array(linhas.into_iter().map(normalizar_metricas).collect()),
        None => normalizar_metricas(linhas.into_iter().next().unwrap_or_default()),
    })
}

fn mediana(ordenados: &[f64]) -> f64 {
    let n = ordenados.len();
    match n {
        0 => 0.0,
        _ if n % 2 == 1 => ordenados[(n - 1) / 2],
        _ => (ordenados[n / 2 - 1] + ordenados[n / 2]) / 2.0,
    }
}

/// `metricasAvancadasSql()`: mediana, sequências, drawdown e PnL semanal/mensal.
fn metricas_avancadas(c: &mut Connection, user_id: &str, raw: &Consulta) -> Result<Value> {
    garantir_metricas(c, user_id)?;
    let f = filtros(c, user_id, raw)?;
    let (u, up) = unidade_stake_sql(&config_stake(c, user_id)?);
    let mut params = up.clone();
    params.extend(f.params.iter().cloned());
    let linhas: Vec<(f64, String, f64)> = {
        let sql = format!(
            "SELECT i.pnl_value AS pnl, i.pnl_ymd AS ymd, i.pnl_value/{u} AS pnlStake
    FROM closed_bets_index i WHERE {} ORDER BY i.pnl_ts ASC, i.platform ASC, i.bet_key ASC",
            f.sql()
        );
        let mut stmt = c.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params.iter()), |r| {
            Ok((
                r.get::<_, Option<f64>>(0)?.unwrap_or(0.0),
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                r.get::<_, Option<f64>>(2)?.unwrap_or(0.0),
            ))
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    // A curva usa fechamentos diários de São Paulo: picos intradiários não
    // entram no drawdown.
    let curva: Vec<(f64, f64)> = {
        let sql = format!(
            "SELECT MAX(i.pnl_ts) AS ts,COALESCE(SUM(i.pnl_value),0) AS pnl
    FROM closed_bets_index i WHERE {} AND i.pnl_ts>0
    GROUP BY i.pnl_ymd ORDER BY i.pnl_ymd ASC",
            f.sql()
        );
        let mut stmt = c.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(f.params.iter()), |r| {
            Ok((r.get::<_, Option<f64>>(0)?.unwrap_or(0.0), r.get::<_, Option<f64>>(1)?.unwrap_or(0.0)))
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };

    let pnls: Vec<f64> = linhas.iter().map(|l| l.0).collect();
    let mut ordenados = pnls.clone();
    ordenados.sort_by(|a, b| a.total_cmp(b));
    let mut ordenados_stake: Vec<f64> = linhas.iter().map(|l| l.2).collect();
    ordenados_stake.sort_by(|a, b| a.total_cmp(b));

    let (mut max_win, mut max_loss, mut cur_win, mut cur_loss) = (0u32, 0u32, 0u32, 0u32);
    for &v in &pnls {
        if v > 0.0 {
            cur_win += 1;
            cur_loss = 0;
            max_win = max_win.max(cur_win);
        } else if v < 0.0 {
            cur_loss += 1;
            cur_win = 0;
            max_loss = max_loss.max(cur_loss);
        } else {
            cur_win = 0;
            cur_loss = 0;
        }
    }

    let (mut equity, mut hwm, mut max_dd, mut vale_ts, mut pico_antes) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
    let mut recuperacao: Option<f64> = None;
    for &(ts, pnl) in &curva {
        equity += pnl;
        if equity >= hwm {
            if max_dd < 0.0 && recuperacao.is_none() && vale_ts != 0.0 && equity >= pico_antes {
                recuperacao = Some(ts);
            }
            hwm = equity;
        } else {
            let dd = equity - hwm;
            if dd < max_dd {
                max_dd = dd;
                vale_ts = ts;
                pico_antes = hwm;
                recuperacao = None;
            }
        }
    }
    let dias_recuperacao = match recuperacao {
        Some(r) if r != 0.0 && vale_ts != 0.0 => js::numero_json(odds::js_round((r - vale_ts) / 86_400_000.0 * 10.0) / 10.0),
        _ => Value::Null,
    };

    let mut por_semana: BTreeMap<String, (f64, f64)> = BTreeMap::new();
    let mut por_mes: BTreeMap<String, (f64, f64)> = BTreeMap::new();
    for (pnl, ymd, stake) in &linhas {
        if !js::e_data_ymd(ymd) {
            continue;
        }
        for (mapa, chave) in [(&mut por_semana, semana_iso(ymd)), (&mut por_mes, ymd[..7].to_string())] {
            let e = mapa.entry(chave).or_insert((0.0, 0.0));
            e.0 += pnl;
            e.1 += stake;
        }
    }
    let serie = |m: BTreeMap<String, (f64, f64)>| -> Value {
        Value::Array(m.into_iter().map(|(d, (v, s))| json!({ "d": d, "v": js::numero_json(v), "vStake": js::numero_json(s) })).collect())
    };
    let n = js::numero_json;
    Ok(json!({
        "medianPnl": n(mediana(&ordenados)),
        "medianPnlStake": n(mediana(&ordenados_stake)),
        "maxWinStreak": max_win,
        "maxLossStreak": max_loss,
        "maxDrawdown": n(max_dd),
        "currentDrawdown": n(equity - hwm),
        "recoveryDays": dias_recuperacao,
        "recovered": recuperacao.is_some(),
        "pnlSemana": serie(por_semana),
        "pnlMes": serie(por_mes)
    }))
}

fn categorias(c: &Connection, user_id: &str, raw: &Consulta) -> Result<Value> {
    let mut sem_categoria = raw.clone();
    sem_categoria.insert("category".into(), "todos".into());
    let f = filtros(c, user_id, &sem_categoria)?;
    let sql = format!(
        "SELECT DISTINCT i.category AS category FROM closed_bets_index i WHERE {} AND i.category<>'' ORDER BY i.category",
        f.sql()
    );
    let mut stmt = c.prepare(&sql)?;
    let lista = stmt.query_map(params_from_iter(f.params.iter()), |r| r.get::<_, String>(0))?;
    Ok(Value::Array(lista.collect::<rusqlite::Result<Vec<_>>>()?.into_iter().map(Value::String).collect()))
}

/// `periodoAnteriorFiltros()`: período de mesma duração imediatamente anterior.
fn periodo_anterior(raw: &Consulta) -> Option<(String, String)> {
    let inicio = primeiro(raw, &["start", "startYmd"], "");
    let fim = primeiro(raw, &["end", "endYmd"], "");
    if !js::e_data_ymd(inicio) && !js::e_data_ymd(fim) {
        return None;
    }
    let hoje = ymd_sao_paulo(hoje_ms());
    let ini = if js::e_data_ymd(inicio) {
        inicio.to_string()
    } else if !fim.is_empty() {
        fim.to_string()
    } else {
        hoje.clone()
    };
    let fim = if js::e_data_ymd(fim) { fim.to_string() } else { hoje };
    let duracao = (diferenca_dias(&ini, &fim) + 1).max(1);
    let fim_anterior = somar_dias(&ini, -1);
    Some((somar_dias(&fim_anterior, -(duracao - 1)), fim_anterior))
}

pub fn stats(c: &mut Connection, user_id: &str, raw: &Consulta) -> Result<Value> {
    let mut raw = raw.clone();
    raw.insert("dateBasis".into(), "realization".into());
    let anterior = if raw.get("includePrevious").is_some_and(|v| v == "1") { periodo_anterior(&raw) } else { None };
    let mut corpo = Map::new();
    corpo.insert("ok".into(), json!(true));
    corpo.insert("total".into(), metricas(c, user_id, &raw, None)?);
    corpo.insert("porSite".into(), metricas(c, user_id, &raw, Some("i.platform"))?);
    corpo.insert("porCategoria".into(), metricas(c, user_id, &raw, Some("i.category"))?);
    if raw.get("includeMarkets").is_some_and(|v| v == "1") {
        corpo.insert("porMercado".into(), metricas(c, user_id, &raw, Some("bet_market_group(i.title_display,i.category)"))?);
    }
    if raw.get("includeSegments").is_some_and(|v| v == "1") {
        corpo.insert(
            "segmentos".into(),
            metricas(c, user_id, &raw, Some("bet_summary_group(i.title_display,i.category,i.odd_value)"))?,
        );
    }
    corpo.insert("avancadas".into(), metricas_avancadas(c, user_id, &raw)?);
    match &anterior {
        Some((ini, fim)) => {
            let mut f = raw.clone();
            f.insert("start".into(), ini.clone());
            f.insert("end".into(), fim.clone());
            corpo.insert("anterior".into(), metricas(c, user_id, &f, None)?);
            corpo.insert("periodoAnterior".into(), json!({ "start": ini, "end": fim }));
        }
        None => {
            corpo.insert("anterior".into(), Value::Null);
            corpo.insert("periodoAnterior".into(), Value::Null);
        }
    }
    corpo.insert("categorias".into(), categorias(c, user_id, &raw)?);
    Ok(Value::Object(corpo))
}

/// `calendarioHistoricoSql()`. `None` quando o mês é inválido.
pub fn calendario(c: &mut Connection, user_id: &str, raw: &Consulta) -> Result<Option<Value>> {
    let mes = raw.get("month").map_or("", |s| s.as_str());
    let b = mes.as_bytes();
    let valido = b.len() == 7
        && b[4] == b'-'
        && b[..4].iter().all(u8::is_ascii_digit)
        && matches!(&mes[5..], "01" | "02" | "03" | "04" | "05" | "06" | "07" | "08" | "09" | "10" | "11" | "12")
        && mes[..4].parse::<i32>().is_ok_and(|a| a >= 1000);
    if !valido {
        return Ok(None);
    }
    let ano: i32 = mes[..4].parse()?;
    let numero: u32 = mes[5..].parse()?;
    let ultimo = NaiveDate::from_ymd_opt(if numero == 12 { ano + 1 } else { ano }, if numero == 12 { 1 } else { numero + 1 }, 1)
        .and_then(|d| d.pred_opt())
        .map_or(28, |d| d.day());
    let mut f = Consulta::new();
    if let Some(p) = raw.get("platform") {
        f.insert("platform".into(), p.clone());
    }
    f.insert("start".into(), format!("{mes}-01"));
    f.insert("end".into(), format!("{mes}-{ultimo}"));
    f.insert("dateBasis".into(), "realization".into());
    let mut dias = match metricas(c, user_id, &f, Some("i.pnl_ymd"))? {
        Value::Array(a) => a,
        _ => vec![],
    };
    dias.sort_by(|a, b| a["chave"].as_str().unwrap_or("").cmp(b["chave"].as_str().unwrap_or("")));
    // Sem data de realização, a aposta não pode ser atribuída a um dia.
    let mut faltando = Consulta::new();
    if let Some(p) = raw.get("platform") {
        faltando.insert("platform".into(), p.clone());
    }
    let mut ff = filtros(c, user_id, &faltando)?;
    ff.onde.push("i.pnl_ymd=''".into());
    let sem_data: i64 = c.query_row(
        &format!("SELECT COUNT(*) FROM closed_bets_index i WHERE {}", ff.sql()),
        params_from_iter(ff.params.iter()),
        |r| r.get(0),
    )?;
    Ok(Some(json!({ "ok": true, "month": mes, "days": dias, "missingDates": sem_data })))
}

/// `GET /api/history/closed/pnl`: séries acumulada, diária e mensal.
pub fn pnl(c: &mut Connection, user_id: &str, raw: &Consulta) -> Result<Value> {
    let modo = raw.get("mode").map_or("accumulated", |s| s.as_str());
    let agora_ts = numero_ou(raw, "now", hoje_ms());
    let periodo = raw.get("period").map_or("all", |s| s.as_str());
    let (mut corte, mut fim) = (0.0f64, agora_ts);
    if periodo == "7" || periodo == "30" {
        corte = agora_ts - periodo.parse::<f64>()? * 86_400_000.0;
    }
    if periodo == "stats" {
        corte = numero_ou(raw, "startTs", 0.0).max(0.0);
        fim = agora_ts.min(numero_ou(raw, "endTs", agora_ts));
    }
    let periodo_calendario = periodo == "7" || periodo == "30";
    let fim_calendario = if periodo_calendario { ymd_sao_paulo(agora_ts) } else { String::new() };
    let inicio_calendario = if periodo_calendario {
        somar_dias(&fim_calendario, -(periodo.parse::<i64>()? - 1))
    } else {
        String::new()
    };
    garantir_metricas(c, user_id)?;

    let mut base = Filtro {
        onde: vec!["i.user_id=?".into(), "i.pnl_ts>0".into(), conta_ativa_sql("i")],
        params: vec![SqlValue::Text(user_id.into())],
    };
    aplicar_periodo_configurado(c, user_id, &mut base)?;
    let casa = raw.get("platform").map_or("todas", |s| if s.is_empty() { "todas" } else { s.as_str() }).to_lowercase();
    if CASAS.contains(&casa.as_str()) {
        base.onde.push("i.platform=?".into());
        base.params.push(SqlValue::Text(casa));
    }
    let onde_sem_data: Vec<String> =
        base.onde.iter().map(|w| if w == "i.pnl_ts>0" { "i.pnl_ts<=0".to_string() } else { w.clone() }).collect();
    let sem_data: i64 = c.query_row(
        &format!("SELECT COUNT(*) FROM closed_bets_index i WHERE {}", onde_sem_data.join(" AND ")),
        params_from_iter(base.params.iter()),
        |r| r.get(0),
    )?;

    let tem_intervalo = periodo == "stats" || periodo_calendario;
    let mut intervalo = Filtro { onde: base.onde.clone(), params: base.params.clone() };
    // Os gráficos diário/mensal só usam o intervalo explícito de Estatísticas.
    if periodo == "stats" {
        if corte > 0.0 {
            intervalo.onde.push("i.pnl_ts>=?".into());
            intervalo.params.push(SqlValue::Real(corte));
        }
        intervalo.onde.push("i.pnl_ts<=?".into());
        intervalo.params.push(SqlValue::Real(fim));
    }
    let eventos_por = |c: &Connection, expr: &str| -> Result<HashMap<String, f64>> {
        let sql = format!(
            "SELECT {expr} AS d,COALESCE(SUM(i.pnl_value),0) AS v
      FROM closed_bets_index i WHERE {} GROUP BY {expr} ORDER BY {expr}",
            intervalo.sql()
        );
        let mut stmt = c.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(intervalo.params.iter()), |r| {
            Ok((r.get::<_, Option<String>>(0)?.unwrap_or_default(), r.get::<_, Option<f64>>(1)?.unwrap_or(0.0)))
        })?;
        Ok(rows.collect::<rusqlite::Result<HashMap<_, _>>>()?)
    };

    if modo == "day" {
        let fim_ymd = ymd_sao_paulo(if periodo == "stats" { fim } else { agora_ts });
        let inicio_ymd = if periodo == "stats" && corte != 0.0 { ymd_sao_paulo(corte) } else { String::new() };
        let por_dia = eventos_por(c, "i.pnl_ymd")?;
        let mut serie = Vec::new();
        if let Some(fim_utc) = utc_dia_ms(&fim_ymd) {
            let inicio_utc = utc_dia_ms(&inicio_ymd).unwrap_or(fim_utc - 29 * 86_400_000);
            let mut t = inicio_utc;
            while t <= fim_utc {
                let d = Utc.timestamp_millis_opt(t).single().map_or_else(String::new, |d| d.format("%Y-%m-%d").to_string());
                serie.push(json!({ "d": d, "v": js::numero_json(por_dia.get(&d).copied().unwrap_or(0.0)) }));
                t += 86_400_000;
            }
        }
        return Ok(json!({ "ok": true, "serie": serie, "missingDates": sem_data }));
    }
    if modo == "month" {
        let por_mes = eventos_por(c, "substr(i.pnl_ymd,1,7)")?;
        let mut chaves: Vec<&String> = por_mes.keys().collect();
        chaves.sort();
        if chaves.is_empty() && periodo != "stats" {
            return Ok(json!({ "ok": true, "serie": [], "missingDates": sem_data }));
        }
        let primeiro_mes = if periodo == "stats" && corte != 0.0 {
            ymd_sao_paulo(corte).get(..7).unwrap_or("").to_string()
        } else {
            chaves.first().map_or(String::new(), |s| s.to_string())
        };
        let ultimo_mes = if periodo == "stats" {
            ymd_sao_paulo(fim).get(..7).unwrap_or("").to_string()
        } else {
            chaves.last().map_or(String::new(), |s| s.to_string())
        };
        let partes = |k: &str| -> (i32, i32) {
            let mut it = k.split('-').map(|p| p.parse::<i32>().unwrap_or(0));
            (it.next().unwrap_or(0), it.next().unwrap_or(0))
        };
        let (mut ano, mut mes) = partes(&primeiro_mes);
        let (ano_fim, mes_fim) = partes(&ultimo_mes);
        let mut serie = Vec::new();
        while ano < ano_fim || (ano == ano_fim && mes <= mes_fim) {
            let k = format!("{ano}-{mes:02}");
            serie.push(json!({ "d": k, "v": js::numero_json(por_mes.get(&k).copied().unwrap_or(0.0)) }));
            mes += 1;
            if mes > 12 {
                mes = 1;
                ano += 1;
            }
        }
        return Ok(json!({ "ok": true, "serie": serie, "missingDates": sem_data }));
    }

    // Série acumulada: um ponto zero logo antes do recorte, valores somados no SQLite.
    let mut acumulado_f = Filtro { onde: base.onde.clone(), params: base.params.clone() };
    let mut inicio_serie = corte;
    if periodo_calendario {
        // Dias de São Paulo inclusivos, como no cartão; não uma janela móvel de horas.
        acumulado_f.onde.push("i.pnl_ymd>=?".into());
        acumulado_f.onde.push("i.pnl_ymd<=?".into());
        acumulado_f.params.push(SqlValue::Text(inicio_calendario.clone()));
        acumulado_f.params.push(SqlValue::Text(fim_calendario.clone()));
        inicio_serie = chrono::DateTime::parse_from_rfc3339(&format!("{inicio_calendario}T00:00:00-03:00"))
            .map_or(f64::NAN, |d| d.timestamp_millis() as f64);
    } else if tem_intervalo {
        if inicio_serie > 0.0 {
            inicio_serie = (inicio_serie - 1.0).max(0.0);
        }
        if inicio_serie > 0.0 {
            acumulado_f.onde.push("i.pnl_ts>=?".into());
            acumulado_f.params.push(SqlValue::Real(inicio_serie));
        }
        acumulado_f.onde.push("i.pnl_ts<=?".into());
        acumulado_f.params.push(SqlValue::Real(fim));
    }
    let eventos: Vec<(f64, f64)> = {
        let sql = format!(
            "SELECT i.pnl_ts AS t,COALESCE(SUM(i.pnl_value),0) AS v FROM closed_bets_index i
      WHERE {} GROUP BY i.pnl_ts ORDER BY i.pnl_ts",
            acumulado_f.sql()
        );
        let mut stmt = c.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(acumulado_f.params.iter()), |r| {
            Ok((r.get::<_, Option<f64>>(0)?.unwrap_or(0.0), r.get::<_, Option<f64>>(1)?.unwrap_or(0.0)))
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut serie: Vec<(f64, f64)> = Vec::new();
    if inicio_serie > 0.0 && !eventos.is_empty() {
        serie.push((inicio_serie, 0.0));
    } else {
        serie.push((eventos.first().map_or(fim, |e| e.0 - 1.0), 0.0));
    }
    let mut acumulado = 0.0;
    for (t, v) in &eventos {
        acumulado += v;
        serie.push((*t, acumulado));
    }
    match serie.last_mut() {
        Some(ultimo) if ultimo.0 == fim => ultimo.1 = acumulado,
        _ => serie.push((fim, acumulado)),
    }
    let mut limpa: Vec<(f64, f64)> = Vec::new();
    for p in serie {
        match limpa.last_mut() {
            Some(u) if u.0 == p.0 => *u = p,
            _ => limpa.push(p),
        }
    }
    let serie: Vec<Value> = limpa.into_iter().map(|(t, v)| json!({ "t": js::numero_json(t), "v": js::numero_json(v) })).collect();
    Ok(json!({ "ok": true, "serie": serie, "missingDates": sem_data }))
}

// ---------------------------------------------------------------------------
// Linhas do ledger
//
// O data_json de uma aposta da Polymarket tem dezenas de KB (metadados de
// todos os mercados do evento). As linhas ficam como texto JSON bruto: só o
// primeiro nível é analisado, e só os campos lidos são decodificados.
// ---------------------------------------------------------------------------

/// Aposta com o primeiro nível analisado e os valores em texto JSON original.
pub type Linha = IndexMap<String, Box<RawValue>>;

fn bruto(v: &Value) -> Box<RawValue> {
    js::bruto(v)
}

/// Decodifica um campo do primeiro nível; `None` quando ausente (undefined).
fn campo(l: &Linha, k: &str) -> Option<Value> {
    l.get(k).and_then(|r| serde_json::from_str(r.get()).ok())
}

fn ler_linha(texto: &str) -> serde_json::Result<Linha> {
    serde_json::from_str(texto)
}

/// `clean()` de `market-title-utils.js`: `String(v || '')` com espaços normalizados.
fn limpar(v: Option<&Value>) -> String {
    if !js::truthy(v) {
        return String::new();
    }
    js::string(v).split_whitespace().collect::<Vec<_>>().join(" ")
}

fn sem_aspas_nas_pontas(s: &str) -> &str {
    let s = s.strip_prefix('"').unwrap_or(s);
    s.strip_suffix('"').unwrap_or(s)
}

/// `a || b` do JavaScript sobre dois campos.
fn ou<'a>(a: Option<&'a Value>, b: Option<&'a Value>) -> Option<&'a Value> {
    if js::truthy(a) { a } else { b }
}

/// Campos de um mercado usados para identificar a aposta e montar os textos.
struct Mercado<'a> {
    bruto: &'a RawValue,
    nulo: bool,
    campos: HashMap<&'static str, Value>,
}

const CAMPOS_MERCADO: [&str; 7] = ["tokenIds", "conditionId", "condition_id", "slug", "question", "groupItemTitle", "sportsMarketType"];

fn ler_mercado(r: &RawValue) -> Mercado<'_> {
    let texto = r.get().trim();
    let mut campos = HashMap::new();
    if let Ok(obj) = serde_json::from_str::<Linha>(texto) {
        for k in CAMPOS_MERCADO {
            if let Some(v) = obj.get(k).and_then(|v| serde_json::from_str(v.get()).ok()) {
                campos.insert(k, v);
            }
        }
    }
    Mercado { bruto: r, nulo: texto == "null", campos }
}

/// `exactMarket()`: o mercado exato da aposta dentro dos metadados do evento.
/// `Err` quando o JS lançaria exceção (a linha é descartada, como no Node).
fn mercado_exato<'m, 'a>(aposta: &Linha, mercados: &'m [Mercado<'a>]) -> Result<Option<&'m Mercado<'a>>, ()> {
    let (asset, asset_id, token) = (campo(aposta, "asset"), campo(aposta, "assetId"), campo(aposta, "tokenId"));
    let asset_bruto = limpar(ou(ou(asset.as_ref(), asset_id.as_ref()), token.as_ref()));
    let asset = sem_aspas_nas_pontas(&asset_bruto).to_string();
    let (cond, cond2) = (campo(aposta, "conditionId"), campo(aposta, "condition_id"));
    let condicao = limpar(ou(cond.as_ref(), cond2.as_ref())).to_lowercase();
    let slug = limpar(campo(aposta, "slug").as_ref()).to_lowercase();

    let valor = |m: &Mercado<'_>, a: &str, b: &str| -> Result<String, ()> {
        if m.nulo {
            return Err(());
        }
        Ok(limpar(ou(m.campos.get(a), m.campos.get(b))))
    };
    let unico = |achados: Vec<&'m Mercado<'a>>| if achados.len() == 1 { Some(achados[0]) } else { None };

    if !asset.is_empty() {
        let mut achados = Vec::new();
        for m in mercados {
            if m.nulo {
                return Err(());
            }
            let ids: &[Value] = match m.campos.get("tokenIds") {
                Some(Value::Array(a)) => a,
                Some(v) if js::truthy(Some(v)) => return Err(()),
                _ => &[],
            };
            if ids.iter().any(|t| sem_aspas_nas_pontas(&limpar(Some(t))) == asset) {
                achados.push(m);
            }
        }
        if let Some(m) = unico(achados) {
            return Ok(Some(m));
        }
    }
    if !condicao.is_empty() {
        let mut achados = Vec::new();
        for m in mercados {
            if valor(m, "conditionId", "condition_id")?.to_lowercase() == condicao {
                achados.push(m);
            }
        }
        if let Some(m) = unico(achados) {
            return Ok(Some(m));
        }
    }
    if !slug.is_empty() {
        let mut achados = Vec::new();
        for m in mercados {
            if valor(m, "slug", "slug")?.to_lowercase() == slug {
                achados.push(m);
            }
        }
        if let Some(m) = unico(achados) {
            return Ok(Some(m));
        }
    }
    Ok(None)
}

/// `compactarLinhaPosicaoFechada()`: a tabela recebe só o mercado exato, não as
/// centenas de mercados irmãos do mesmo evento. Nunca altera o ledger.
fn compactar(mut aposta: Linha) -> Result<Linha, ()> {
    let Some(meta_bruto) = aposta.get("_sportMeta") else { return Ok(aposta) };
    // Meta que não é objeto (ou sem `markets` em array) volta sem mudança.
    let Ok(mut meta) = serde_json::from_str::<Linha>(meta_bruto.get()) else { return Ok(aposta) };
    let Some(Ok(lista)) = meta.get("markets").map(|m| serde_json::from_str::<Vec<Box<RawValue>>>(m.get())) else {
        return Ok(aposta);
    };
    let mercados: Vec<Mercado<'_>> = lista.iter().map(|r| ler_mercado(r)).collect();
    let proprio = mercado_exato(&aposta, &mercados)?;

    let mut textos_mercado: HashSet<String> = HashSet::new();
    for m in &mercados {
        if m.nulo {
            return Err(());
        }
        for k in ["slug", "question", "groupItemTitle", "sportsMarketType"] {
            let v = m.campos.get(k);
            if js::truthy(v) {
                textos_mercado.insert(js::string(v).to_lowercase());
            }
        }
    }
    let lista_meta = |k: &str| meta.get(k).and_then(|r| serde_json::from_str::<Value>(r.get()).ok());
    let textos_evento: Vec<Value> = match lista_meta("eventTexts") {
        Some(Value::Array(a)) => a,
        _ => match lista_meta("texts") {
            Some(Value::Array(a)) => a.into_iter().filter(|v| !textos_mercado.contains(&js::string(Some(v)).to_lowercase())).collect(),
            _ => vec![],
        },
    };
    let mut textos: Vec<Value> = Vec::new();
    let do_proprio = proprio
        .iter()
        .flat_map(|p| ["slug", "question", "groupItemTitle", "sportsMarketType"].map(|k| p.campos.get(k).cloned().unwrap_or(Value::Null)));
    for v in textos_evento.iter().cloned().chain(do_proprio) {
        if js::truthy(Some(&v)) && !textos.contains(&v) {
            textos.push(v);
        }
    }
    let proprio_bruto: Vec<&RawValue> = proprio.map(|p| p.bruto).into_iter().collect();
    let markets = js::bruto(&proprio_bruto);
    drop(mercados);
    meta.insert("eventTexts".into(), bruto(&Value::Array(textos_evento)));
    meta.insert("texts".into(), bruto(&Value::Array(textos)));
    meta.insert("markets".into(), markets);
    let novo = js::bruto(&meta);
    aposta.insert("_sportMeta".into(), novo);
    Ok(aposta)
}

/// O `_sportMeta` que a tabela recebe de `compactar`, como texto. `None` quando
/// a linha seria descartada (onde o JS lançaria exceção). O ledger usa isto
/// para conferir que reduzir os mercados gravados não muda o que a tabela mostra.
pub(crate) fn meta_da_tabela(aposta: &Value) -> Option<String> {
    let compacta = compactar(ler_linha(&js::para_texto(aposta)).ok()?).ok()?;
    Some(compacta.get("_sportMeta").map_or_else(String::new, |r| r.get().to_string()))
}

fn chave_kalshi(aposta: &Linha) -> Option<Value> {
    ["slug", "ticker", "market_ticker"].iter().filter_map(|k| campo(aposta, k)).find(|v| js::truthy(Some(v)))
}

fn e_kalshi(aposta: &Linha) -> bool {
    let p = campo(aposta, "_platform");
    js::truthy(p.as_ref()) && js::string(p.as_ref()).to_lowercase() == "kalshi"
}

/// `lerMetadadosKalshiPersistidos()`, com só o primeiro nível analisado.
fn metadados_kalshi(c: &Connection, tipo: &str, chaves: Vec<String>) -> Result<HashMap<String, Linha>> {
    let mut unicas: Vec<String> = Vec::new();
    let mut vistas = HashSet::new();
    for k in chaves {
        let k = k.trim().to_string();
        if !k.is_empty() && vistas.insert(k.clone()) {
            unicas.push(k);
        }
    }
    let mut achados = HashMap::new();
    for lote in unicas.chunks(400) {
        let marcas = vec!["?"; lote.len()].join(",");
        let sql = format!("SELECT cache_key,data_json FROM kalshi_metadata_cache WHERE kind=? AND cache_key IN ({marcas})");
        let mut stmt = c.prepare(&sql)?;
        let params: Vec<&dyn ToSql> = std::iter::once(&tipo as &dyn ToSql).chain(lote.iter().map(|s| s as &dyn ToSql)).collect();
        let rows = stmt.query_map(params.as_slice(), |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (k, data) = row?;
            if let Ok(v) = ler_linha(&data) {
                achados.insert(k, v);
            }
        }
    }
    Ok(achados)
}

static VS_JOGO: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)\s+(?:vs\.?|versus|@)\s+").expect("regex"));

fn texto_truthy(v: Option<&Value>) -> String {
    if js::truthy(v) { js::string(v) } else { String::new() }
}

/// `anexarMetadadosTitulosKalshi()`: anexa metadados de apresentação às linhas
/// Kalshi, sem alterar o ledger.
pub(crate) fn anexar_metadados_kalshi(c: &Connection, mut linhas: Vec<Linha>) -> Result<Vec<Linha>> {
    let candidatas: Vec<usize> = (0..linhas.len()).filter(|&i| e_kalshi(&linhas[i])).collect();
    if candidatas.is_empty() {
        return Ok(linhas);
    }
    let chaves = candidatas.iter().map(|&i| texto_truthy(chave_kalshi(&linhas[i]).as_ref())).collect();
    let mercados = metadados_kalshi(c, "market", chaves)?;
    let eventos = metadados_kalshi(c, "event", mercados.values().map(|m| texto_truthy(campo(m, "event_ticker").as_ref())).collect())?;
    for i in candidatas {
        let row = &mut linhas[i];
        // O Map do JS é consultado com o valor original (sem trim nem conversão).
        let Some(mercado) = chave_kalshi(row).as_ref().and_then(Value::as_str).and_then(|k| mercados.get(k)) else { continue };
        let evento = campo(mercado, "event_ticker").as_ref().and_then(Value::as_str).and_then(|k| eventos.get(k));
        let titulo_evento = evento.map_or(String::new(), |e| {
            texto_truthy(campo(e, "title").as_ref()).split(':').next().unwrap_or("").trim().to_string()
        });
        let titulo_bruto = texto_truthy(campo(row, "title").as_ref()).split(" - ").next().unwrap_or("").trim().to_string();
        let confronto = if VS_JOGO.is_match(&titulo_evento) {
            Value::String(titulo_evento)
        } else if let Some(v) = evento.and_then(|e| campo(e, "_teamsTitle")).filter(|v| js::truthy(Some(v))) {
            v
        } else if let Some(v) = campo(row, "_matchupKey").filter(|v| js::truthy(Some(v))) {
            v
        } else if VS_JOGO.is_match(&titulo_bruto) {
            Value::String(titulo_bruto)
        } else {
            Value::String(String::new())
        };
        let mut m = Map::new();
        for k in ["title", "subtitle", "yes_sub_title", "no_sub_title", "rules_primary"] {
            if let Some(v) = campo(mercado, k) {
                m.insert(k.into(), v);
            }
        }
        let escopo = evento
            .and_then(|e| campo(e, "product_metadata").and_then(|p| p.get("competition_scope").cloned()).filter(|v| js::truthy(Some(v))))
            .or_else(|| evento.and_then(|e| campo(e, "title")).filter(|v| js::truthy(Some(v))))
            .unwrap_or_else(|| Value::String(String::new()));
        row.insert("_kalshiTitleMeta".into(), bruto(&json!({ "market": m, "scope": escopo, "matchup": confronto })));
    }
    Ok(linhas)
}

/// A consulta da ordem do ledger. Apostas com o mesmo `first_seen_at` saem na
/// ordem do índice que o SQLite escolhe: um índice novo em
/// `closed_bets_ledger` pode mudá-la (ver `db::INDICES_DA_MANUTENCAO`).
fn sql_ordem_ledger(com_titulos: bool) -> String {
    format!(
        "SELECT l.rowid{} FROM closed_bets_ledger l
    {}
    WHERE l.user_id=? AND {} ORDER BY l.first_seen_at ASC",
        if com_titulos { ",i.title_display" } else { "" },
        if com_titulos {
            "LEFT JOIN closed_bets_index i ON i.user_id=l.user_id AND i.platform=l.platform AND i.bet_key=l.bet_key"
        } else {
            ""
        },
        conta_ativa_sql("l")
    )
}

/// `readClosedLedger()`: todo o histórico da conta ativa, na ordem de entrada.
pub(crate) fn ler_ledger(c: &Connection, user_id: &str, com_titulos: bool) -> Result<Vec<Linha>> {
    // Mesma consulta do Node, mas a ordenação carrega só rowid e título; o
    // data_json é lido depois, na ordem obtida.
    let sql = sql_ordem_ledger(com_titulos);
    let ordem: Vec<(i64, Option<String>)> = {
        let mut stmt = c.prepare(&sql)?;
        let rows = stmt.query_map([user_id], |r| Ok((r.get(0)?, if com_titulos { r.get(1)? } else { None })))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut stmt = c.prepare_cached("SELECT data_json FROM closed_bets_ledger WHERE rowid=?")?;
    let mut saida = Vec::with_capacity(ordem.len());
    for (rowid, titulo) in ordem {
        let data: String = stmt.query_row([rowid], |r| r.get(0))?;
        let Ok(mut aposta) = ler_linha(&data) else { continue };
        if com_titulos {
            aposta.insert("_titleDisplay".into(), bruto(&titulo.map_or(Value::Null, Value::String)));
        }
        saida.push(aposta);
    }
    anexar_metadados_kalshi(c, saida)
}

/// Candidatas da migração preguiçosa de `atualizarModalidadesKalshiNoLedger()`:
/// linhas Kalshi do `readClosedLedger()` que ainda guardam a categoria raiz
/// "Sports", sem `_sportMetaChecked` e com `_eventTicker`. Vêm da mais recente
/// para a mais antiga (`_settledTime`, senão `_startDate`), com ordenação
/// estável sobre a ordem do ledger, e no máximo `limite` linhas.
pub(crate) fn pendentes_modalidade_kalshi(c: &Connection, user_id: &str, limite: usize) -> Result<Vec<Value>> {
    // Mesma ordem do `ler_ledger`; o data_json só é lido para as linhas da
    // Kalshi (a coluna `platform` é o `casaHistorico(_platform)` da linha).
    let sql = format!(
        "SELECT l.rowid,l.platform FROM closed_bets_ledger l
    WHERE l.user_id=? AND {} ORDER BY l.first_seen_at ASC",
        conta_ativa_sql("l")
    );
    let ordem: Vec<i64> = {
        let mut stmt = c.prepare(&sql)?;
        let rows = stmt.query_map([user_id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        rows.filter_map(|r| match r {
            Ok((rowid, casa)) if casa == "kalshi" => Some(Ok(rowid)),
            Ok(_) => None,
            Err(e) => Some(Err(e)),
        })
        .collect::<rusqlite::Result<_>>()?
    };
    let texto_minusculo = |v: Option<Value>| texto_truthy(v.as_ref()).to_lowercase();
    let mut stmt = c.prepare_cached("SELECT data_json FROM closed_bets_ledger WHERE rowid=?")?;
    let mut candidatas: Vec<(f64, Value)> = Vec::new();
    for rowid in ordem {
        let data: String = stmt.query_row([rowid], |r| r.get(0))?;
        let Ok(aposta) = ler_linha(&data) else { continue };
        let pendente = texto_minusculo(campo(&aposta, "_platform")) == "kalshi"
            && texto_minusculo(campo(&aposta, "sport")) == "sports"
            && !js::truthy(campo(&aposta, "_sportMetaChecked").as_ref())
            && js::truthy(campo(&aposta, "_eventTicker").as_ref());
        if !pendente {
            continue;
        }
        // `Date.parse(_settledTime || _startDate || '') || 0`.
        let quando = [campo(&aposta, "_settledTime"), campo(&aposta, "_startDate")]
            .into_iter()
            .find(|v| js::truthy(v.as_ref()))
            .flatten()
            .and_then(|v| js::data_ms(&js::string(Some(&v))))
            .unwrap_or(0.0);
        if let Ok(linha) = serde_json::from_str::<Value>(&data) {
            candidatas.push((quando, linha));
        }
    }
    candidatas.sort_by(|a, b| b.0.total_cmp(&a.0));
    Ok(candidatas.into_iter().take(limite).map(|(_, linha)| linha).collect())
}

pub fn ledger(c: &Connection, user_id: &str) -> Result<Vec<u8>> {
    #[derive(Serialize)]
    struct Corpo {
        ok: bool,
        fechados: Vec<Linha>,
    }
    Ok(js::para_vec(&Corpo { ok: true, fechados: ler_ledger(c, user_id, false)? }))
}

/// Resposta do `POST /api/history/closed` legado: `{ok, count, fechados}`.
pub fn ledger_legado(c: &Connection, user_id: &str) -> Result<Vec<u8>> {
    #[derive(Serialize)]
    struct Corpo {
        ok: bool,
        count: usize,
        fechados: Vec<Linha>,
    }
    let fechados = ler_ledger(c, user_id, false)?;
    Ok(js::para_vec(&Corpo { ok: true, count: fechados.len(), fechados }))
}

pub fn exportar(c: &Connection, user_id: &str) -> Result<Vec<u8>> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Corpo<'a> {
        ok: bool,
        exported_at: String,
        source: &'a str,
        count: usize,
        fechados: Vec<Linha>,
    }
    let fechados = ler_ledger(c, user_id, true)?;
    Ok(js::para_vec(&Corpo {
        ok: true,
        exported_at: agora(),
        source: "closed_bets_ledger",
        count: fechados.len(),
        fechados,
    }))
}

pub fn status_indice(c: &Connection, user_id: &str) -> Result<Value> {
    let contar = |sql: &str| -> rusqlite::Result<i64> { c.query_row(sql, [user_id], |r| r.get(0)) };
    let ledger = contar("SELECT COUNT(*) FROM closed_bets_ledger WHERE user_id=?")?;
    let indexado = contar("SELECT COUNT(*) FROM closed_bets_index WHERE user_id=?")?;
    let pendente = contar(
        "SELECT COUNT(*) FROM closed_bets_ledger l LEFT JOIN closed_bets_index i
    ON i.user_id=l.user_id AND i.platform=l.platform AND i.bet_key=l.bet_key
    WHERE l.user_id=? AND (i.bet_key IS NULL OR i.metrics_ready=0)",
    )?;
    Ok(json!({
        "ok": true,
        "ledger": ledger,
        "indexed": indexado,
        "pending": pendente,
        "complete": ledger == indexado && pendente == 0,
        "versions": crate::db::versoes_indice(c, user_id)?
    }))
}

/// `GET .../index/pending`. `None` quando o cursor é inválido.
pub fn pendentes_indice(c: &Connection, user_id: &str, raw: &Consulta) -> Result<Option<Vec<u8>>> {
    let cursor = match raw.get("cursor").filter(|s| !s.is_empty()) {
        None => 0.0,
        Some(s) => js::number(Some(&Value::String(s.clone()))),
    };
    if !(cursor.is_finite() && cursor.fract() == 0.0 && cursor.abs() <= 9_007_199_254_740_991.0 && cursor >= 0.0) {
        return Ok(None);
    }
    let mut stmt = c.prepare(
        "SELECT l.rowid AS cursor,l.data_json FROM closed_bets_ledger l LEFT JOIN closed_bets_index i
      ON i.user_id=l.user_id AND i.platform=l.platform AND i.bet_key=l.bet_key
      WHERE l.user_id=? AND l.rowid>? AND (i.bet_key IS NULL OR i.metrics_ready=0)
      ORDER BY l.rowid LIMIT 200",
    )?;
    let rows = stmt.query_map(rusqlite::params![user_id, cursor as i64], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
    let rows: Vec<(i64, String)> = rows.collect::<rusqlite::Result<_>>()?;
    let proximo = rows.last().map(|r| r.0);
    // JSON inválido é erro, nunca uma linha descartada em silêncio.
    let fechados = rows.iter().map(|(_, d)| ler_linha(d)).collect::<Result<Vec<Linha>, _>>()?;
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Corpo {
        ok: bool,
        fechados: Vec<Linha>,
        next_cursor: Option<i64>,
    }
    Ok(Some(js::para_vec(&Corpo { ok: true, fechados: anexar_metadados_kalshi(c, fechados)?, next_cursor: proximo })))
}

fn ordenacao(raw: &Consulta) -> String {
    let sort = raw.get("sort").map_or("inicio-asc", |s| s.as_str());
    let dir = raw.get("dir").map_or("", |s| s.as_str());
    match sort {
        "inicio-asc" => "i.pnl_ts DESC,i.platform ASC,i.bet_key ASC".into(),
        "data" => format!("i.pnl_ts {},i.platform ASC,i.bet_key ASC", if dir == "desc" { "DESC" } else { "ASC" }),
        "titulo-asc" | "titulo" => {
            format!("i.title_display COLLATE NOCASE {},i.platform ASC,i.bet_key ASC", if dir == "desc" { "DESC" } else { "ASC" })
        }
        "odd" | "stake" | "pnl" => {
            let coluna = match sort {
                "odd" => "i.odd_value",
                "stake" => "i.stake_value",
                _ => "i.pnl_value",
            };
            format!("{coluna} {},i.platform ASC,i.bet_key ASC", if dir == "asc" { "ASC" } else { "DESC" })
        }
        _ => "i.pnl_ts DESC,i.platform ASC,i.bet_key ASC".into(),
    }
}

fn titulos_layout(c: &Connection, f: &Filtro) -> Result<Value> {
    let mut stmt = c.prepare(&format!("SELECT DISTINCT i.title_display AS title FROM closed_bets_index i WHERE {}", f.sql()))?;
    let lista = stmt.query_map(params_from_iter(f.params.iter()), |r| r.get::<_, Option<String>>(0))?;
    Ok(Value::Array(lista.map(|t| t.map(|t| Value::String(t.unwrap_or_default()))).collect::<rusqlite::Result<_>>()?))
}

/// `GET /api/history/closed/query`: uma página do histórico filtrado.
pub fn consultar(c: &mut Connection, user_id: &str, raw: &Consulta) -> Result<Vec<u8>> {
    garantir_metricas(c, user_id)?;
    let mut raw = raw.clone();
    raw.insert("dateBasis".into(), "realization".into());
    let f = filtros(c, user_id, &raw)?;
    let pagina = numero_ou(&raw, "page", 1.0).max(1.0);
    let tamanho = numero_ou(&raw, "pageSize", 20.0).clamp(10.0, 50.0);
    let deslocamento = (pagina - 1.0) * tamanho;
    let total: i64 = c.query_row(
        &format!("SELECT COUNT(*) FROM closed_bets_index i WHERE {}", f.sql()),
        params_from_iter(f.params.iter()),
        |r| r.get(0),
    )?;
    // Mesma consulta do Node, mas a ordenação carrega só as chaves; o
    // data_json é lido depois, pela chave primária.
    let sql = format!(
        "SELECT i.platform,i.bet_key,i.title_display,i.pnl_ts FROM closed_bets_index i JOIN closed_bets_ledger l ON l.user_id=i.user_id AND l.platform=i.platform AND l.bet_key=i.bet_key
      WHERE {} ORDER BY {} LIMIT ? OFFSET ?",
        f.sql(),
        ordenacao(&raw)
    );
    let mut params = f.params.clone();
    params.push(SqlValue::Real(tamanho));
    params.push(SqlValue::Real(deslocamento));
    let chaves: Vec<(String, String, Option<String>, Option<f64>)> = {
        let mut stmt = c.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params.iter()), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut itens = Vec::with_capacity(chaves.len());
    {
        let mut stmt =
            c.prepare_cached("SELECT data_json FROM closed_bets_ledger WHERE user_id=? AND platform=? AND bet_key=?")?;
        for (platform, bet_key, titulo, ts) in chaves {
            let data: String = stmt.query_row(rusqlite::params![user_id, platform, bet_key], |r| r.get(0))?;
            let Ok(mut aposta) = ler_linha(&data) else { continue };
            aposta.insert("_titleDisplay".into(), bruto(&titulo.map_or(Value::Null, Value::String)));
            aposta.insert("_positionClosedTs".into(), bruto(&js::numero_json(ts.filter(|t| !t.is_nan()).unwrap_or(0.0))));
            if let Ok(compacta) = compactar(aposta) {
                itens.push(compacta);
            }
        }
    }
    let itens = anexar_metadados_kalshi(c, itens)?;
    let layout = if raw.get("layout").is_some_and(|v| v == "1") { Some(titulos_layout(c, &f)?) } else { None };
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Corpo {
        ok: bool,
        total: i64,
        page: Value,
        page_size: Value,
        fechados: Vec<Linha>,
        #[serde(skip_serializing_if = "Option::is_none")]
        layout_titles: Option<Value>,
    }
    Ok(js::para_vec(&Corpo {
        ok: true,
        total,
        page: js::numero_json(pagina),
        page_size: js::numero_json(tamanho),
        fechados: itens,
        layout_titles: layout,
    }))
}

pub fn layout(c: &Connection, user_id: &str, raw: &Consulta) -> Result<Value> {
    let mut raw = raw.clone();
    raw.insert("dateBasis".into(), "realization".into());
    let f = filtros(c, user_id, &raw)?;
    Ok(json!({ "ok": true, "layoutTitles": titulos_layout(c, &f)? }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pendentes_de_modalidade_kalshi() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = crate::db::abrir(&dir.path().join("t.db")).unwrap();
        let user = crate::db::usuario_local(&mut c).unwrap().id;
        let inserir = |chave: &str, casa: &str, visto: &str, d: Value| {
            c.execute(
                "INSERT INTO closed_bets_ledger(user_id,platform,bet_key,data_json,first_seen_at,updated_at) VALUES(?,?,?,?,?,?)",
                rusqlite::params![user, casa, chave, d.to_string(), visto, visto],
            )
            .unwrap();
        };
        let k = |ticker: &str, quando: &str| json!({ "_platform": "kalshi", "sport": "Sports", "_eventTicker": ticker, "_settledTime": quando });
        inserir("a", "kalshi", "2026-01-01", k("EV-A", "2026-10-01T00:00:00Z"));
        inserir("b", "kalshi", "2026-01-02", k("EV-B", "2026-11-01T00:00:00Z"));
        // Empate de data: fica a ordem do ledger (first_seen_at).
        inserir("c", "kalshi", "2026-01-03", k("EV-C", "2026-10-01T00:00:00Z"));
        // Sem data conta como 0 e vai para o fim; `_startDate` substitui `_settledTime`.
        inserir("d", "kalshi", "2026-01-04", json!({ "_platform": "Kalshi", "sport": "SPORTS", "_eventTicker": "EV-D" }));
        inserir("e", "kalshi", "2026-01-05", json!({ "_platform": "kalshi", "sport": "Sports", "_eventTicker": "EV-E", "_startDate": "2026-12-01" }));
        // Fora: já checada, outra modalidade, sem evento, outra casa.
        inserir("f", "kalshi", "2026-01-06", json!({ "_platform": "kalshi", "sport": "Sports", "_eventTicker": "EV-F", "_sportMetaChecked": true }));
        inserir("g", "kalshi", "2026-01-07", json!({ "_platform": "kalshi", "sport": "Soccer", "_eventTicker": "EV-G" }));
        inserir("h", "kalshi", "2026-01-08", json!({ "_platform": "kalshi", "sport": "Sports", "_eventTicker": "" }));
        inserir("i", "polymarket", "2026-01-09", json!({ "_platform": "polymarket", "sport": "Sports", "_eventTicker": "EV-I" }));
        let tickers = |limite| -> Vec<String> {
            pendentes_modalidade_kalshi(&c, &user, limite).unwrap().iter().map(|l| l["_eventTicker"].as_str().unwrap().to_string()).collect()
        };
        assert_eq!(tickers(12), ["EV-E", "EV-B", "EV-A", "EV-C", "EV-D"]);
        assert_eq!(tickers(2), ["EV-E", "EV-B"]);
    }

    #[test]
    fn indices_da_manutencao_nao_mudam_os_planos_do_ledger() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = crate::db::abrir(&dir.path().join("t.db")).unwrap();
        let user = crate::db::usuario_local(&mut c).unwrap().id;
        for i in 0..50 {
            let casa = if i % 2 == 0 { "polymarket" } else { "kalshi" };
            let d = if i % 3 == 0 { json!({ "slug": i }) } else { json!({ "slug": i, "_confirmedClosedTs": 1 }) };
            c.execute(
                "INSERT INTO closed_bets_ledger(user_id,platform,bet_key,data_json,first_seen_at,updated_at) VALUES(?,?,?,?,'2026-01-01','x')",
                rusqlite::params![user, casa, format!("k{i}"), d.to_string()],
            )
            .unwrap();
        }
        let consultas = [
            sql_ordem_ledger(false),
            sql_ordem_ledger(true),
            format!("SELECT l.rowid,l.platform FROM closed_bets_ledger l WHERE l.user_id=? AND {} ORDER BY l.first_seen_at ASC", conta_ativa_sql("l")),
            "SELECT bet_key,data_json FROM closed_bets_ledger WHERE user_id=? AND platform='polymarket' AND bet_key IN ('k0','k2')".into(),
        ];
        let planos = |c: &Connection| -> Vec<Vec<String>> {
            consultas
                .iter()
                .map(|q| {
                    let mut s = c.prepare(&format!("EXPLAIN QUERY PLAN {q}")).unwrap();
                    s.query_map([&user], |r| r.get::<_, String>(3)).unwrap().map(Result::unwrap).collect()
                })
                .collect()
        };
        let ordem = |c: &Connection| ler_ledger(c, &user, false).unwrap().iter().map(|l| l["slug"].get().to_string()).collect::<Vec<_>>();
        let (antes, ordem_antes) = (planos(&c), ordem(&c));
        crate::db::criar_indices_da_manutencao(&c).unwrap();
        assert_eq!(planos(&c), antes, "o índice parcial não entra em outras consultas");
        assert_eq!(ordem(&c), ordem_antes, "a ordem do ledger com empates não muda");
    }

    /// Mede as etapas da exportação num banco real (cópia).
    #[test]
    #[ignore]
    fn medir_etapas_exportacao() {
        use std::time::Instant;
        let c = crate::db::abrir(std::path::Path::new(&std::env::var("BETTY_COMPARAR_DB").unwrap())).unwrap();
        let t = Instant::now();
        let textos: Vec<String> = {
            let mut s = c.prepare("SELECT data_json FROM closed_bets_ledger ORDER BY rowid").unwrap();
            s.query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap()
        };
        let mb = textos.iter().map(String::len).sum::<usize>() as f64 / 1e6;
        eprintln!("ler {} linhas ({mb:.0} MB): {:?}", textos.len(), t.elapsed());
        let t = Instant::now();
        let valores: Vec<Value> = textos.iter().map(|s| serde_json::from_str(s).unwrap()).collect();
        eprintln!("parse Value: {:?}", t.elapsed());
        let t = Instant::now();
        let _ = serde_json::to_vec(&valores).unwrap();
        eprintln!("serializar Value: {:?}", t.elapsed());
        let t = Instant::now();
        let brutos: Vec<HashMap<String, Box<serde_json::value::RawValue>>> =
            textos.iter().map(|s| serde_json::from_str(s).unwrap()).collect();
        eprintln!("parse só o primeiro nível (RawValue): {:?}", t.elapsed());
        let t = Instant::now();
        drop(valores);
        eprintln!("liberar Value: {:?}", t.elapsed());
        drop(brutos);

        let user: String = c.query_row("SELECT id FROM users LIMIT 1", [], |r| r.get(0)).unwrap();
        let t = Instant::now();
        let linhas = ler_ledger(&c, &user, true).unwrap();
        eprintln!("ler_ledger ({} linhas): {:?}", linhas.len(), t.elapsed());
        drop(linhas);
        let t = Instant::now();
        let b = exportar(&c, &user).unwrap();
        eprintln!("exportar + serializar ({} MB): {:?}", b.len() / 1_000_000, t.elapsed());
    }

    #[test]
    fn datas_em_sao_paulo() {
        // 2026-11-26 01:30 UTC ainda é dia 25 em São Paulo (UTC-3).
        assert_eq!(ymd_sao_paulo(1_795_656_600_000.0), "2026-11-25");
        assert_eq!(ymd_sao_paulo(0.0), "");
        assert_eq!(semana_iso("2026-01-01"), "2026-W01");
        assert_eq!(semana_iso("2027-01-01"), "2026-W53");
        assert_eq!(somar_dias("2026-03-01", -1), "2026-02-28");
        assert_eq!(diferenca_dias("2026-01-01", "2026-01-31"), 30);
    }

    #[test]
    fn periodo_anterior_tem_mesma_duracao() {
        let mut c = Consulta::new();
        c.insert("start".into(), "2026-02-01".into());
        c.insert("end".into(), "2026-02-28".into());
        assert_eq!(periodo_anterior(&c), Some(("2026-01-04".into(), "2026-01-31".into())));
        assert_eq!(periodo_anterior(&Consulta::new()), None);
    }

    #[test]
    fn consulta_da_url() {
        let c = ler_consulta(Some("platform=kalshi&title=a%20b+c&x=1&x=2"));
        assert_eq!(c["title"], "a b c");
        assert_eq!(c["x"], "1,2");
        assert_eq!(primeiro(&c, &["site", "platform"], "todas"), "kalshi");
    }
}
