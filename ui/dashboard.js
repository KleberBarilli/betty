document.body.append(document.getElementById('syncProgress'));
// A wallet Polymarket nunca é exposta ao navegador. O backend identifica a conta pela sessão.
const KALSHI_PROXY = "/api/kalshi";
// Polymarket: backend próprio do Betty. Nenhum Worker externo é usado.
const POLY_API = "/api/polymarket";
const GAMMA_API = "/api/polymarket-gamma";
const CACHE_KEY = "betty_pos_cache_v3";
const LAST_NONEMPTY_CACHE_KEY = "betty_pos_last_nonempty_v1";
const LEGACY_CACHE_KEYS = ["betty_pos_cache_v2"];
const CACHE_TTL_MS = 30 * 60 * 1000; // 30 min — cache stale-while-revalidate; dados antigos continuam visíveis enquanto atualiza
const DATE_CACHE_KEY = "betty_date_cache_v1";
const DATE_CACHE_TTL_MS = 7 * 24 * 60 * 60 * 1000; // 7 dias — datas de jogo
const GAMMA_DATE_META_CACHE_KEY = "betty_gamma_date_meta_v1";
const PAGE_SIZE_KEY = 'betty_page_size_v1';
let pageSize = Number(localStorage.getItem(PAGE_SIZE_KEY) || 20);
if (![10,20,50].includes(pageSize)) pageSize = 20;
function itensPorPagina() { return pageSize; }
function alterarItensPorPagina(v) {
const n = Number(v);
if (![10,20,50].includes(n)) return;
pageSize = n;
try { localStorage.setItem(PAGE_SIZE_KEY, String(n)); } catch (_) {}
paginaAtual = 1;
aplicarFiltros({ manterPagina: true });
}
// Unidade de stake (editável em Configurações), separada por casa.
const STAKE_UNIDADE_DEFAULT = 1.00; // $ por 1 u (padrão)
// Saldo real da Kalshi, obtido diretamente da API. Mantemos o último valor confirmado durante refreshs.
let KALSHI_API_BALANCE = null;
// Saldo pUSD real da wallet Polymarket, consultado on-chain pelo backend.
// Nunca usamos o histórico para preencher o valor exibido no cabeçalho.
let POLY_API_BALANCE = null;
let PREDICT_API_BALANCE = null;
let SXBET_API_BALANCE = null;
let saldoPredictFalhou = false;
const CASAS_NOVAS = { predict: "Predict.fun", sxbet: "SX Bet" };
const NOMES_CASAS = { polymarket: "Polymarket", kalshi: "Kalshi", ...CASAS_NOVAS };
const CASAS_STAKE = Object.keys(NOMES_CASAS);
function normalizarRegrasStake(regras) {
return Array.isArray(regras) ? regras
.filter(r => r && r.from && typeof r.unidade === "number" && isFinite(r.unidade) && r.unidade > 0)
.map(r => ({ from: String(r.from).slice(0, 10), unidade: Math.round(Number(r.unidade) * 100) / 100 }))
.sort((a, b) => a.from.localeCompare(b.from)) : [];
}
function normalizarConfigStake(config, fallback) {
const unidade = Number(config && config.unidade);
return {
unidade: Number.isFinite(unidade) && unidade > 0 ? Math.round(unidade * 100) / 100 : fallback,
regras: normalizarRegrasStake(config && config.regras)
};
}
function criarConfiguracaoStakePadrao(unidade = STAKE_UNIDADE_DEFAULT, regras = []) {
return Object.fromEntries(CASAS_STAKE.map(casa => [casa, { unidade, regras: normalizarRegrasStake(regras) }]));
}
let STAKE_POR_CASA = criarConfiguracaoStakePadrao();
function criarPeriodosFechadasPadrao(fallback = { inicio: "", fim: "" }) {
return Object.fromEntries(CASAS_STAKE.map(casa => [casa, { ...fallback }]));
}
let PERIODO_FECHADAS_CONFIGURADO = criarPeriodosFechadasPadrao();
let calendarioPeriodoFechadasConfigMes = new Date();
let calendarioPeriodoFechadasConfigAberto = false;
let casaCalendarioPeriodoFechadasConfig = "polymarket";
let campoCalendarioPeriodoFechadasConfig = "start";
function ymdConfigurado(valor) {
const ymd = String(valor || "").slice(0, 10);
return /^\d{4}-\d{2}-\d{2}$/.test(ymd) ? ymd : "";
}
function normalizarPeriodoFechadas(inicio, fim) {
const periodo = { inicio: ymdConfigurado(inicio), fim: ymdConfigurado(fim) };
if (periodo.inicio && periodo.fim && periodo.inicio > periodo.fim) return null;
return periodo;
}
function normalizarPeriodosFechadasPorCasa(periodos, fallback) {
const origem = periodos && typeof periodos === "object" ? periodos : {};
const possuiConfiguracaoPorCasa = CASAS_STAKE.some(casa => origem[casa] && typeof origem[casa] === "object");
return Object.fromEntries(CASAS_STAKE.map(casa => [casa,
normalizarPeriodoFechadas(
possuiConfiguracaoPorCasa ? origem[casa]?.inicio : fallback?.inicio,
possuiConfiguracaoPorCasa ? origem[casa]?.fim : fallback?.fim
) || { inicio: "", fim: "" }
]));
}
function periodoFechadasDaCasa(casa) {
return PERIODO_FECHADAS_CONFIGURADO[casaDaAposta(casa)] || { inicio: "", fim: "" };
}
function chavePeriodoFechadasConfigurado() {
return CASAS_STAKE.map(casa => `${periodoFechadasDaCasa(casa).inicio}|${periodoFechadasDaCasa(casa).fim}`).join("|");
}
function limparCachesPeriodoFechadas() {
resumoDashboardDb = null;
resumoDashboardDbChave = "";
resumoPeriodoDash = null;
resumoEstatDb = null;
resumoEstatDbChave = "";
pnlHistoricoDb = null;
}
function casaDaAposta(aposta) {
const casa = String(typeof aposta === "string" ? aposta : aposta?._platform || "polymarket").toLowerCase();
return CASAS_STAKE.includes(casa) ? casa : "polymarket";
}
function configStakeDaCasa(casa) {
return STAKE_POR_CASA[casaDaAposta(casa)] || { unidade: STAKE_UNIDADE_DEFAULT, regras: [] };
}
// Estas configurações pertencem à conta autenticada.
// O servidor identifica o usuário pela sessão e salva tudo em settings.user_id.
const CONFIG_SYNC_URL = "/api/settings";
function aplicarObjetoConfiguracao(o) {
if (!o || typeof o !== "object") return false;
const unidadeLegada = Number(o.stakeUnidade);
const fallback = Number.isFinite(unidadeLegada) && unidadeLegada > 0 ? Math.round(unidadeLegada * 100) / 100 : STAKE_UNIDADE_DEFAULT;
const regrasLegadas = normalizarRegrasStake(o.stakeRegras);
const porCasa = o.stakePorCasa && typeof o.stakePorCasa === "object" ? o.stakePorCasa : {};
STAKE_POR_CASA = Object.fromEntries(CASAS_STAKE.map(casa => [casa,
normalizarConfigStake(porCasa[casa], fallback)
]));
for (const casa of CASAS_STAKE) {
if (!porCasa[casa]) STAKE_POR_CASA[casa].regras = normalizarRegrasStake(regrasLegadas);
}
const periodoLegado = normalizarPeriodoFechadas(o.periodoFechadasInicio, o.periodoFechadasFim) || { inicio: "", fim: "" };
PERIODO_FECHADAS_CONFIGURADO = normalizarPeriodosFechadasPorCasa(o.periodoFechadasPorCasa, periodoLegado);
return true;
}
async function lerConfiguracaoRemota() {
try {
const res = await fetch(CONFIG_SYNC_URL + "?t=" + Date.now(), {
method: "GET",
cache: "no-store",
headers: { "Accept": "application/json" }
});
if (!res.ok) return null;
const data = await res.json();
return data && data.config ? data.config : null;
} catch (e) {
return null;
}
}
async function salvarConfiguracaoRemota() {
try {
const res = await fetch(CONFIG_SYNC_URL, {
method: "POST",
cache: "no-store",
headers: { "Content-Type": "application/json", "Accept": "application/json" },
body: JSON.stringify({
// Mantidos também para compatibilidade com versões anteriores do app.
stakeUnidade: Number(configStakeDaCasa("polymarket").unidade),
stakeRegras: configStakeDaCasa("polymarket").regras.map(r => ({ from: String(r.from).slice(0, 10), unidade: Number(r.unidade) })),
stakePorCasa: Object.fromEntries(CASAS_STAKE.map(casa => {
const config = configStakeDaCasa(casa);
return [casa, { unidade: Number(config.unidade), regras: config.regras.map(r => ({ from: String(r.from).slice(0, 10), unidade: Number(r.unidade) })) }];
})),
periodoFechadasPorCasa: Object.fromEntries(CASAS_STAKE.map(casa => [casa, periodoFechadasDaCasa(casa)]))
})
});
return res.ok;
} catch (e) {
return false;
}
}
async function sincronizarConfiguracaoRemota() {
const remoto = await lerConfiguracaoRemota();
if (!remoto) return false;
aplicarObjetoConfiguracao(remoto);
limparCachesPeriodoFechadas();
preencherFormConfiguracao();
renderizarStats();
if (typeof desenharGrafico === "function") try { desenharGrafico(); } catch (e) {}
if (typeof renderEstatisticas === "function" && viewAtual === "estatisticas") renderEstatisticas();
return true;
}
// Unidade vigente de uma casa em um timestamp (ms).
function stakeUnidadeEm(ts, casa) {
const config = configStakeDaCasa(casa);
let u = config.unidade;
if (!ts || !config.regras.length) return u > 0 ? u : 1;
const ymd = ymdBrasil(new Date(ts));
for (const r of config.regras) {
if (ymd && r.from <= ymd) u = r.unidade;
}
return u > 0 ? u : 1;
}
function dolarParaUnidades(valorDolar, ts, casa) {
const u = stakeUnidadeEm(ts, casa);
return u > 0 ? (Number(valorDolar) || 0) / u : (Number(valorDolar) || 0);
}
function renderStakeRulesList(casa) {
const config = configStakeDaCasa(casa);
const el = document.getElementById("stakeRulesList-" + casa);
if (!el) return;
if (!config.regras.length) {
el.innerHTML = '<div style="font-size:0.78rem;color:var(--muted-2);padding:4px 0;">Nenhuma regra — vale só o stake padrão.</div>';
return;
}
el.innerHTML = config.regras.map((r, i) => `
<div class="stake-rule" data-i="${i}">
<div>
<div class="rule-lbl">A partir de</div>
<input type="date" value="${r.from}" data-on-change="atualizarRegraStake('${casa}',${i},'from',this.value)" />
</div>
<div>
<div class="rule-lbl">1 u ($)</div>
<input type="number" step="0.01" min="0.01" value="${Number(r.unidade).toFixed(2)}" data-on-change="atualizarRegraStake('${casa}',${i},'unidade',this.value)" />
</div>
<button type="button" class="btn-del" title="Remover" data-on-click="removerRegraStake('${casa}',${i})">×</button>
</div>
`).join("");
}
function adicionarRegraStake(casa) {
const config = configStakeDaCasa(casa);
const hoje = ymdBrasil(new Date()) || new Date().toISOString().slice(0, 10);
config.regras.push({ from: hoje, unidade: config.unidade });
config.regras.sort((a, b) => a.from.localeCompare(b.from));
renderStakeRulesList(casa);
}
function atualizarRegraStake(casa, i, campo, valor) {
const config = configStakeDaCasa(casa);
if (!config.regras[i]) return;
if (campo === "from") {
config.regras[i].from = String(valor).slice(0, 10);
config.regras.sort((a, b) => a.from.localeCompare(b.from));
renderStakeRulesList(casa);
} else if (campo === "unidade") {
const n = parseFloat(valor);
if (isFinite(n) && n > 0) config.regras[i].unidade = Math.round(n * 100) / 100;
}
}
function removerRegraStake(casa, i) {
configStakeDaCasa(casa).regras.splice(i, 1);
renderStakeRulesList(casa);
}
function idCampoPeriodoFechadas(casa, campo, sufixo = "") {
return `cfgClosedRange${campo === "start" ? "Start" : "End"}-${casa}${sufixo}`;
}
function inputPeriodoFechadas(casa, campo) {
return document.getElementById(idCampoPeriodoFechadas(casa, campo));
}
function preencherFormConfiguracao() {
for (const casa of CASAS_STAKE) {
const input = document.getElementById("cfgStakeUnidade-" + casa);
if (input) input.value = Number(configStakeDaCasa(casa).unidade).toFixed(2);
renderStakeRulesList(casa);
}
for (const casa of CASAS_STAKE) {
const periodo = periodoFechadasDaCasa(casa);
const inicio = inputPeriodoFechadas(casa, "start");
const fim = inputPeriodoFechadas(casa, "end");
if (inicio) inicio.value = periodo.inicio;
if (fim) fim.value = periodo.fim;
}
atualizarBotoesPeriodoFechadasConfigurado();
}
function limparPeriodoFechadasConfigurado(casaAlvo = null) {
for (const casa of casaAlvo ? [casaDaAposta(casaAlvo)] : CASAS_STAKE) {
const inicio = inputPeriodoFechadas(casa, "start");
const fim = inputPeriodoFechadas(casa, "end");
if (inicio) inicio.value = "";
if (fim) fim.value = "";
}
atualizarBotoesPeriodoFechadasConfigurado();
}
function atualizarBotoesPeriodoFechadasConfigurado() {
const texto = valor => valor ? valor.split("-").reverse().join("/") : "dd/mm/aaaa";
for (const casa of CASAS_STAKE) for (const campo of ["start", "end"]) {
const input = inputPeriodoFechadas(casa, campo);
const valor = document.getElementById(idCampoPeriodoFechadas(casa, campo, "Value"));
const botao = document.getElementById(idCampoPeriodoFechadas(casa, campo, "Button"));
if (valor) valor.textContent = texto(input?.value || "");
if (botao) botao.classList.remove("active");
}
}
function toggleCalendarioPeriodoFechadasConfigurado(casa, campo, ev) {
if (ev) ev.stopPropagation();
const popup = document.getElementById("configClosedRangePopup");
if (!popup) return;
if (calendarioPeriodoFechadasConfigAberto && casaCalendarioPeriodoFechadasConfig === casa && campoCalendarioPeriodoFechadasConfig === campo) {
fecharCalendarioPeriodoFechadasConfigurado();
return;
}
campoCalendarioPeriodoFechadasConfig = campo;
casaCalendarioPeriodoFechadasConfig = casaDaAposta(casa);
const input = inputPeriodoFechadas(casaCalendarioPeriodoFechadasConfig, campo);
const selecionada = input?.value || "";
if (selecionada) {
const [ano, mes] = selecionada.split("-").map(Number);
calendarioPeriodoFechadasConfigMes = new Date(ano, mes - 1, 1);
} else {
const hoje = new Date();
calendarioPeriodoFechadasConfigMes = new Date(hoje.getFullYear(), hoje.getMonth(), 1);
}
renderCalendarioPeriodoFechadasConfigurado();
calendarioPeriodoFechadasConfigAberto = true;
popup.classList.add("open");
const botaoAtivo = document.getElementById(idCampoPeriodoFechadas(casaCalendarioPeriodoFechadasConfig, campo, "Button"));
botaoAtivo?.setAttribute("aria-expanded", "true");
botaoAtivo?.classList.add("open");
requestAnimationFrame(posicionarCalendarioPeriodoFechadasConfigurado);
document.addEventListener("scroll", fecharCalendarioPeriodoFechadasConfigurado, true);
window.addEventListener("resize", posicionarCalendarioPeriodoFechadasConfigurado);
setTimeout(() => document.addEventListener("click", fecharCalendarioPeriodoFechadasConfiguradoFora), 0);
}
function fecharCalendarioPeriodoFechadasConfiguradoFora(ev) {
const popup = document.getElementById("configClosedRangePopup");
const botao = document.getElementById(idCampoPeriodoFechadas(casaCalendarioPeriodoFechadasConfig, campoCalendarioPeriodoFechadasConfig, "Button"));
if (popup && !popup.contains(ev.target) && botao && !botao.contains(ev.target)) fecharCalendarioPeriodoFechadasConfigurado();
}
function fecharCalendarioPeriodoFechadasConfigurado() {
const popup = document.getElementById("configClosedRangePopup");
if (popup) { popup.classList.remove("open"); popup.style.top = ""; popup.style.left = ""; }
for (const casa of CASAS_STAKE) for (const campo of ["start", "end"]) {
const botao = document.getElementById(idCampoPeriodoFechadas(casa, campo, "Button"));
botao?.setAttribute("aria-expanded", "false");
botao?.classList.remove("open");
}
calendarioPeriodoFechadasConfigAberto = false;
document.removeEventListener("click", fecharCalendarioPeriodoFechadasConfiguradoFora);
document.removeEventListener("scroll", fecharCalendarioPeriodoFechadasConfigurado, true);
window.removeEventListener("resize", posicionarCalendarioPeriodoFechadasConfigurado);
}
function posicionarCalendarioPeriodoFechadasConfigurado() {
const popup = document.getElementById("configClosedRangePopup");
const botao = document.getElementById(idCampoPeriodoFechadas(casaCalendarioPeriodoFechadasConfig, campoCalendarioPeriodoFechadasConfig, "Button"));
if (!popup || !botao || !popup.classList.contains("open")) return;
const r = botao.getBoundingClientRect(), margem = 8, espacamento = 8;
const largura = popup.offsetWidth || 260;
let esquerda = r.left, topo = r.bottom + espacamento;
if (esquerda + largura > window.innerWidth - margem) esquerda = window.innerWidth - largura - margem;
if (esquerda < margem) esquerda = margem;
popup.style.left = `${esquerda}px`;
popup.style.top = `${topo}px`;
}
function navegarMesCalendarioPeriodoFechadasConfigurado(delta, ev) {
if (ev) ev.stopPropagation();
calendarioPeriodoFechadasConfigMes = new Date(calendarioPeriodoFechadasConfigMes.getFullYear(), calendarioPeriodoFechadasConfigMes.getMonth() + delta, 1);
renderCalendarioPeriodoFechadasConfigurado();
}
function renderCalendarioPeriodoFechadasConfigurado() {
const popup = document.getElementById("configClosedRangePopup");
if (!popup) return;
const ano = calendarioPeriodoFechadasConfigMes.getFullYear(), mes = calendarioPeriodoFechadasConfigMes.getMonth();
const nomesMes = ["janeiro", "fevereiro", "março", "abril", "maio", "junho", "julho", "agosto", "setembro", "outubro", "novembro", "dezembro"];
const primeiroDia = new Date(ano, mes, 1).getDay(), diasNoMes = new Date(ano, mes + 1, 0).getDate(), diasMesAnterior = new Date(ano, mes, 0).getDate();
const pad2 = n => String(n).padStart(2, "0"), hoje = ymdBrasil(new Date());
const input = inputPeriodoFechadas(casaCalendarioPeriodoFechadasConfig, campoCalendarioPeriodoFechadasConfig);
const selecionada = input?.value || "";
let celulas = "";
for (let i = primeiroDia - 1; i >= 0; i--) celulas += `<div class="day other">${diasMesAnterior - i}</div>`;
for (let dia = 1; dia <= diasNoMes; dia++) {
const ymd = `${ano}-${pad2(mes + 1)}-${pad2(dia)}`;
let classe = "day";
if (ymd === hoje) classe += " today";
if (ymd === selecionada) classe += " selected";
celulas += `<div class="${classe}" data-on-click="selecionarDiaPeriodoFechadasConfigurado('${ymd}',event)">${dia}</div>`;
}
const resto = (primeiroDia + diasNoMes) % 7;
if (resto) for (let dia = 1; dia <= 7 - resto; dia++) celulas += `<div class="day other">${dia}</div>`;
popup.innerHTML = `<div class="date-popup-head"><span>${nomesMes[mes]} de ${ano}</span><div class="date-popup-nav"><button type="button" data-on-click="navegarMesCalendarioPeriodoFechadasConfigurado(-1,event)">‹</button><button type="button" data-on-click="navegarMesCalendarioPeriodoFechadasConfigurado(1,event)">›</button></div></div><div class="date-grid"><div class="dow">D</div><div class="dow">S</div><div class="dow">T</div><div class="dow">Q</div><div class="dow">Q</div><div class="dow">S</div><div class="dow">S</div>${celulas}</div><div class="date-popup-foot"><button type="button" class="limpar" data-on-click="limparCampoPeriodoFechadasConfigurado(event)">Limpar</button><button type="button" data-on-click="selecionarDiaPeriodoFechadasConfigurado('${hoje}',event)">Hoje</button></div>`;
}
function selecionarDiaPeriodoFechadasConfigurado(ymd, ev) {
if (ev) ev.stopPropagation();
const input = inputPeriodoFechadas(casaCalendarioPeriodoFechadasConfig, campoCalendarioPeriodoFechadasConfig);
if (input) input.value = ymd;
atualizarBotoesPeriodoFechadasConfigurado();
fecharCalendarioPeriodoFechadasConfigurado();
}
function limparCampoPeriodoFechadasConfigurado(ev) {
if (ev) ev.stopPropagation();
const input = inputPeriodoFechadas(casaCalendarioPeriodoFechadasConfig, campoCalendarioPeriodoFechadasConfig);
if (input) input.value = "";
atualizarBotoesPeriodoFechadasConfigurado();
fecharCalendarioPeriodoFechadasConfigurado();
}
function salvarConfiguracao() {
const msg = document.getElementById("configMsg");
const periodos = {};
for (const casa of CASAS_STAKE) {
const periodo = normalizarPeriodoFechadas(inputPeriodoFechadas(casa, "start")?.value, inputPeriodoFechadas(casa, "end")?.value);
if (!periodo) {
if (msg) { msg.textContent = "A data inicial não pode ser posterior à data final."; msg.className = "config-msg err"; }
return;
}
periodos[casa] = periodo;
}
for (const casa of CASAS_STAKE) {
const config = configStakeDaCasa(casa);
const input = document.getElementById("cfgStakeUnidade-" + casa);
const unidade = parseFloat(input && input.value);
if (!isFinite(unidade) || unidade <= 0) {
if (msg) { msg.textContent = "A unidade de stake da " + NOMES_CASAS[casa] + " deve ser > 0."; msg.className = "config-msg err"; }
return;
}
if (config.regras.some(r => !r.from || !isFinite(r.unidade) || r.unidade <= 0)) {
if (msg) { msg.textContent = "Regras de stake inválidas (data e valor > 0)."; msg.className = "config-msg err"; }
return;
}
config.unidade = Math.round(unidade * 100) / 100;
config.regras = normalizarRegrasStake(config.regras);
}
PERIODO_FECHADAS_CONFIGURADO = periodos;
limparCachesPeriodoFechadas();
salvarConfiguracaoRemota().then(function(ok) {
if (!ok) {
const m = document.getElementById("configMsg");
if (m) {
m.textContent = "Não foi possível salvar no servidor. Verifique sua conexão.";
m.className = "config-msg err";
}
}
});
preencherFormConfiguracao();
renderizarStats();
if (typeof desenharGrafico === "function") try { desenharGrafico(); } catch (e) {}
if (viewAtual === "estatisticas") renderEstatisticas();
if (abaAtual === "fechado") aplicarFiltros({ manterPagina: false });
if (msg) { msg.textContent = "Configurações atualizadas."; msg.className = "config-msg ok"; }
}
function restaurarConfiguracao() {
STAKE_POR_CASA = criarConfiguracaoStakePadrao();
PERIODO_FECHADAS_CONFIGURADO = criarPeriodosFechadasPadrao();
limparCachesPeriodoFechadas();
salvarConfiguracaoRemota().catch(function() {});
preencherFormConfiguracao();
renderizarStats();
if (typeof desenharGrafico === "function") try { desenharGrafico(); } catch (e) {}
if (viewAtual === "estatisticas") renderEstatisticas();
if (abaAtual === "fechado") aplicarFiltros({ manterPagina: false });
const msg = document.getElementById("configMsg");
if (msg) { msg.textContent = "Restaurado para o padrão."; msg.className = "config-msg ok"; }
}
// A configuração é carregada do servidor para a conta autenticada.
sincronizarConfiguracaoRemota().catch(function() {});
function lerCacheLocal(permitirExpirado) {
const keys = [CACHE_KEY].concat(LEGACY_CACHE_KEYS || []);
for (const key of keys) {
try {
const raw = localStorage.getItem(key);
if (!raw) continue;
const o = JSON.parse(raw);
if (!o || !Array.isArray(o.abertos)) continue;
if (!permitirExpirado && Date.now() - (o.t || 0) > CACHE_TTL_MS) continue;
// O histórico fechado pertence ao SQLite. Migra snapshots antigos para uma
// versão enxuta, preservando somente as posições abertas necessárias para
// manter a tela estável enquanto a API externa atualiza.
const enxuto = { t: Number(o.t) || 0, abertos: o.abertos, fechados: [] };
if (key !== CACHE_KEY || (o.fechados || []).length) {
try { localStorage.setItem(CACHE_KEY, JSON.stringify(enxuto)); } catch (_) {}
}
return enxuto;
} catch (e) {}
}
return null;
}
// Retry para rede instável / Safari "Load failed"
async function fetchComRetry(url, opts, tentativas) {
tentativas = tentativas || 3;
var ultimoErr = null;
for (var i = 0; i < tentativas; i++) {
try {
var res = await fetch(url, opts);
if (/^\/api\/kalshi\/(?:positions|settlements|balance)(?:\?|$)/.test(String(url)) &&
(!opts?.method || opts.method === 'GET') && [502, 503, 504].includes(res.status) && i + 1 < tentativas) {
await res.body?.cancel();
await new Promise(function (r) { setTimeout(r, 400 * (i + 1)); });
continue;
}
return res;
} catch (e) {
ultimoErr = e;
await new Promise(function (r) { setTimeout(r, 400 * (i + 1)); });
}
}
throw ultimoErr || new Error("Falha de rede");
}
function salvarCacheLocal() {
try {
const snapshot = {
t: Date.now(),
// O snapshot precisa conservar a posição inteira para a tela iniciar estável,
// mas o metadado Gamma contém payloads extensos que não são necessários fora
// da normalização. Persistimos somente a sua forma compacta.
abertos: compactarPosicoesParaCache((abertos || []).filter(d => !Object.hasOwn(CASAS_NOVAS, casaDaAposta(d)))),
// Fechadas não ficam no localStorage: o ledger SQLite é a fonte durável e
// paginada desse histórico, mesmo após reiniciar o aplicativo.
fechados: [],
};
// Nunca transformamos um snapshot bom em um cache vazio.
// A API pode responder [] momentaneamente durante reload/indexação.
const anterior = lerCacheLocal(true);
const temDados = snapshot.abertos.length;
const tinhaDados = anterior && (anterior.abertos || []).length;
const finalSnapshot = (!temDados && tinhaDados) ? anterior : snapshot;
const serializedSnapshot=JSON.stringify(snapshot);
localStorage.setItem(CACHE_KEY, finalSnapshot===snapshot ? serializedSnapshot : JSON.stringify(finalSnapshot));
// Backup durável somente das posições abertas. As fechadas já estão
// protegidas pelo ledger SQLite e não devem duplicar o histórico no navegador.
if (temDados) {
localStorage.setItem(LAST_NONEMPTY_CACHE_KEY, serializedSnapshot);
}
} catch (e) { registrarFalhaPersistenciaCache(CACHE_KEY, e); }
}
function lerUltimoSnapshotNaoVazio() {
try {
const raw = localStorage.getItem(LAST_NONEMPTY_CACHE_KEY);
if (!raw) return null;
const o = JSON.parse(raw);
if (!o || !Array.isArray(o.abertos) || !o.abertos.length) return null;
return { t: Number(o.t) || 0, abertos: o.abertos, fechados: [] };
} catch (_) {
return null;
}
}
// Último saldo confirmado de cada casa, para a abertura com dados salvos.
// Fica numa chave própria: uma conta sem posições abertas também tem saldo.
const SALDOS_SALVOS_KEY = "betty:saldosSalvos:v1";
const VARIAVEIS_SALDO = {
polymarket: [() => POLY_API_BALANCE, v => { POLY_API_BALANCE = v; }],
kalshi: [() => KALSHI_API_BALANCE, v => { KALSHI_API_BALANCE = v; }],
predict: [() => PREDICT_API_BALANCE, v => { PREDICT_API_BALANCE = v; }],
sxbet: [() => SXBET_API_BALANCE, v => { SXBET_API_BALANCE = v; }]
};
function lerSaldosSalvos() {
try {
const o = JSON.parse(localStorage.getItem(SALDOS_SALVOS_KEY) || "null");
return o && typeof o === "object" ? o : {};
} catch (_) { return {}; }
}
function salvarSaldosLocais() {
const anteriores = lerSaldosSalvos(), saldos = {};
for (const [casa, [ler]] of Object.entries(VARIAVEIS_SALDO)) {
if (!BETTY_CONNECTIONS?.[casa]) continue;
const valor = ler();
// Um saldo que ainda é o salvo (a consulta desta vez falhou) mantém a data dele.
if (saldosDosDadosSalvos.has(casa)) { if (anteriores[casa]) saldos[casa] = anteriores[casa]; }
else if (Number.isFinite(valor)) saldos[casa] = { v: valor, t: Date.now() };
}
try { localStorage.setItem(SALDOS_SALVOS_KEY, JSON.stringify(saldos)); } catch (_) {}
}
let saldosSalvosDaAbertura = {};
function lerCacheDatas() {
try {
const raw = localStorage.getItem(DATE_CACHE_KEY);
if (!raw) return {};
const o = JSON.parse(raw);
if (!o || !o.map || Date.now() - (o.t || 0) > DATE_CACHE_TTL_MS) return {};
return o.map;
} catch (e) {
return {};
}
}
function salvarCacheDatas(map) {
try {
localStorage.setItem(DATE_CACHE_KEY, JSON.stringify({ t: Date.now(), map: map }));
} catch (e) { registrarFalhaPersistenciaCache(DATE_CACHE_KEY, e); }
}
let metasDatasPnl = null;
function lerCacheMetasDatasGamma() {
try {
const raw = localStorage.getItem(GAMMA_DATE_META_CACHE_KEY);
if (!raw) return {};
const o = JSON.parse(raw);
if (!o || o.v !== GAMMA_DATE_META_CACHE_VERSION || !o.map || Date.now() - (o.t || 0) > DATE_CACHE_TTL_MS) return {};
return o.map;
} catch (e) { return {}; }
}
function salvarCacheMetasDatasGamma(map) {
metasDatasPnl = map;
try {
localStorage.setItem(GAMMA_DATE_META_CACHE_KEY, JSON.stringify({ v: GAMMA_DATE_META_CACHE_VERSION, t: Date.now(), map: compactarMetasDatasGammaParaCache(map) }));
} catch (e) { registrarFalhaPersistenciaCache(GAMMA_DATE_META_CACHE_KEY, e); }
}
// Metadados de esporte vindos do Gamma/Polymarket.
// O endpoint de eventos expõe category/subcategory/tags/series/markets,
// que são muito mais confiáveis do que tentar adivinhar pelo "vs.".
const SPORT_META_CACHE_KEY = "betty_sport_meta_v3";
const SPORT_META_CACHE_TTL_MS = 7 * 24 * 60 * 60 * 1000;
const SPORT_META_CACHE_VERSION = 3;
const GAMMA_DATE_META_CACHE_VERSION = 2;
const avisosFalhaCachePersistente = new Set();
function registrarFalhaPersistenciaCache(chave, erro) {
if (avisosFalhaCachePersistente.has(chave)) return;
avisosFalhaCachePersistente.add(chave);
console.warn(`[Betty] O cache local \"${chave}\" não pôde ser persistido; os dados atuais continuam em memória.`, erro?.name || erro);
}
function lerCacheEsportes() {
try {
const raw = localStorage.getItem(SPORT_META_CACHE_KEY);
if (!raw) return {};
const o = JSON.parse(raw);
if (!o || o.v !== SPORT_META_CACHE_VERSION || !o.map || Date.now() - (o.t || 0) > SPORT_META_CACHE_TTL_MS) return {};
return o.map;
} catch (e) { return {}; }
}
function salvarCacheEsportes(map) {
try {
localStorage.setItem(SPORT_META_CACHE_KEY, JSON.stringify({ v: SPORT_META_CACHE_VERSION, t: Date.now(), map: compactarMetasEsporteParaCache(map) }));
} catch (e) { registrarFalhaPersistenciaCache(SPORT_META_CACHE_KEY, e); }
}
function tokenIdsDoMercado(m) {
const out = [];
const add = v => { if (v != null && String(v).trim()) out.push(String(v).trim()); };
for (const field of [m?.clobTokenIds, m?.tokenIds]) {
  if (Array.isArray(field)) { for (const v of field) add(v); }
  else if (typeof field === "string" && field.trim()) {
    try {
      const parsed = JSON.parse(field);
      if (Array.isArray(parsed)) for (const v of parsed) add(v);
      else add(field);
    } catch (_) {
      for (const v of field.split(",")) add(v);
    }
  }
}
if (Array.isArray(m?.tokens)) {
  for (const t of m.tokens) add(t?.token_id ?? t?.tokenId ?? t?.id);
}
return [...new Set(out)];
}
function extrairMetaEsporteEvento(event) {
if (!event) return null;
const texts = [];
const add = v => { if (v != null && String(v).trim()) texts.push(String(v).trim()); };
add(event.category); add(event.subcategory); add(event.title); add(event.subtitle);
add(event.slug); add(event.ticker); add(event.seriesSlug);
for (const t of (Array.isArray(event.tags) ? event.tags : [])) {
add(t?.label); add(t?.slug);
}
for (const c of (Array.isArray(event.categories) ? event.categories : [])) {
add(c?.label); add(c?.slug); add(c?.parentCategory);
}
for (const sr of (Array.isArray(event.series) ? event.series : [])) {
add(sr?.title); add(sr?.subtitle); add(sr?.slug); add(sr?.ticker); add(sr?.description);
for (const t of (Array.isArray(sr?.tags) ? sr.tags : [])) { add(t?.label); add(t?.slug); }
for (const c of (Array.isArray(sr?.categories) ? sr.categories : [])) { add(c?.label); add(c?.slug); }
}
const eventTexts = [...new Set(texts)];
const sportsMarkets = [];
const markets = [];
for (const m of (Array.isArray(event.markets) ? event.markets : [])) {
add(m?.category); add(m?.sportsMarketType); add(m?.slug); add(m?.question); add(m?.groupItemTitle);
if (m?.sportsMarketType) sportsMarkets.push(String(m.sportsMarketType));
for (const t of (Array.isArray(m?.tags) ? m.tags : [])) { add(t?.label); add(t?.slug); }
for (const c of (Array.isArray(m?.categories) ? m.categories : [])) { add(c?.label); add(c?.slug); }
markets.push({
conditionId: m?.conditionId || m?.condition_id || null,
slug: m?.slug || null,
question: m?.question || null,
groupItemTitle: m?.groupItemTitle || null,
description: m?.description || null,
sportsMarketType: m?.sportsMarketType || null,
line: m?.line ?? m?.groupItemThreshold ?? null,
category: m?.category || null,
// Token IDs identify the exact YES/NO or Over/Under market held.
tokenIds: tokenIdsDoMercado(m)
});
}
return {
texts: [...new Set(texts)],
eventTexts,
sportsMarketTypes: [...new Set(sportsMarkets)],
markets,
eventId: event.id || null,
seriesSlug: event.seriesSlug || null
};
}
// O Gamma envia descrições e objetos auxiliares muito grandes. A normalização
// usa apenas os identificadores do mercado, pergunta/tipo/linha e os textos
// de classificação; por isso esta é a única forma que vai ao localStorage.
function compactarMetaEsporteParaCache(meta) {
if (!meta || typeof meta !== "object") return null;
const texto = v => v == null || String(v).trim() === "" ? null : String(v).trim();
const unique = itens => [...new Set((Array.isArray(itens) ? itens : []).map(texto).filter(Boolean))];
const markets = (Array.isArray(meta.markets) ? meta.markets : []).map(m => ({
conditionId: texto(m?.conditionId || m?.condition_id),
slug: texto(m?.slug),
question: texto(m?.question),
groupItemTitle: texto(m?.groupItemTitle),
sportsMarketType: texto(m?.sportsMarketType),
line: m?.line ?? null,
category: texto(m?.category),
tokenIds: unique(m?.tokenIds)
}));
return {
texts: unique(meta.texts),
eventTexts: Array.isArray(meta.eventTexts) ? unique(meta.eventTexts) : null,
sportsMarketTypes: unique(meta.sportsMarketTypes),
markets,
eventId: texto(meta.eventId),
seriesSlug: texto(meta.seriesSlug)
};
}
function compactarMetasEsporteParaCache(map) {
return Object.fromEntries(Object.entries(map || {}).map(([slug, meta]) => [slug, compactarMetaEsporteParaCache(meta)]).filter(([, meta]) => meta));
}
function compactarPosicoesParaCache(lista) {
return (lista || []).map(d => d?._sportMeta ? { ...d, _sportMeta: compactarMetaEsporteParaCache(d._sportMeta) } : d);
}
function compactarMetasDatasGammaParaCache(map) {
const campos = obj => ({
startTime: obj?.startTime || null, startDate: obj?.startDate || null,
gameStartTime: obj?.gameStartTime || null, eventDate: obj?.eventDate || null,
endDate: obj?.endDate || null, closedTime: obj?.closedTime || null,
umaEndDate: obj?.umaEndDate || null, creationDate: obj?.creationDate || null,
acceptingOrdersTimestamp: obj?.acceptingOrdersTimestamp || null,
title: obj?.title || ""
});
return Object.fromEntries(Object.entries(map || {}).map(([slug, meta]) => [slug, {
event: campos(meta?.event),
markets: Object.fromEntries(Object.entries(meta?.markets || {}).map(([marketSlug, market]) => [marketSlug, campos(market)]))
}]));
}
// Mantido como função de compatibilidade; nenhuma aposta é excluída.
function filtrarExcluidos(lista) {
return lista || [];
}
let abaAtual = "aberto";
let abertos = [];
const syncExportAt = Object.fromEntries(CASAS_STAKE.map(casa => [casa, ""]));
let fechados = [];
let carregado = { aberto: false, fechado: false };
// Ordenação por clique no cabeçalho (null = usa o select)
let sortCol = null;   // data | titulo | odd | stake | pnl
let sortDir = "asc";  // asc | desc
let filtroDataDia = null; // 'YYYY-MM-DD' ou null
let filtroCasaAtual = "todas";
let filtroEsportePosicoesAtual = "todos";
let calendarioMes = new Date();
let calendarioAberto = false;
let casaPopupAberto = false;
let listaFiltradaAtual = [];
let paginaAtual = 1;
let historicoFechadoIndexado = false;
// Durante a escrita do índice (especialmente em uma migração de categoria),
// ele é atualizado em lotes. Nenhuma tela financeira pode publicar uma
// leitura feita entre esses lotes. O contador também invalida respostas HTTP
// que foram iniciadas antes do índice terminar.
let historicoIndiceEmSincronizacao = false;
let versaoIndiceHistorico = 0;
function iniciarSincronizacaoIndiceHistorico() {
historicoIndiceEmSincronizacao = true;
versaoIndiceHistorico += 1;
return versaoIndiceHistorico;
}
function concluirSincronizacaoIndiceHistorico(versao) {
if (versao !== versaoIndiceHistorico) return false;
historicoIndiceEmSincronizacao = false;
return true;
}
// Abertura com dados salvos: as telas mostram logo o último estado gravado (o
// índice no SQLite e as posições e saldos do cache local), com o selo
// "Atualizando" no topo, até a sincronização com as casas confirmar tudo.
// Nesse período as leituras do histórico saem de um estado salvo congelado no
// backend (`/api/history/snapshot`, `snapshot=1`): mesmo com a sincronização
// gravando o índice em lotes, nenhuma tela vê uma leitura pela metade.
let exibindoDadosSalvos = false;
let indiceSalvoDisponivel = false;
let dadosSalvosEm = 0;
const saldosDosDadosSalvos = new Set();
const VERSAO_DADOS_SALVOS = -1;
function versaoLeituraHistorico() {
return exibindoDadosSalvos ? VERSAO_DADOS_SALVOS : versaoIndiceHistorico;
}
function indiceDisponivel() {
return exibindoDadosSalvos ? indiceSalvoDisponivel : historicoFechadoIndexado;
}
function indiceHistoricoLegivel() {
return exibindoDadosSalvos ? indiceSalvoDisponivel : historicoFechadoIndexado && !historicoIndiceEmSincronizacao;
}
function urlLeituraHistorico(url) {
return exibindoDadosSalvos ? url + (url.includes("?") ? "&" : "?") + "snapshot=1" : url;
}
function leituraHistoricoAindaAtual(versao) {
if (versao === VERSAO_DADOS_SALVOS) return exibindoDadosSalvos;
return !historicoIndiceEmSincronizacao && versao === versaoIndiceHistorico;
}
function resumoHistoricoPodeSerPublicado(versao) {
if (versao === VERSAO_DADOS_SALVOS) return exibindoDadosSalvos && indiceSalvoDisponivel;
return historicoFechadoIndexado && leituraHistoricoAindaAtual(versao);
}
// O cache do navegador existe apenas para a tabela abrir rápido. Ele pode ser
// anterior ao último refresh e não é uma fonte financeira. Cards, estatísticas
// e gráficos só podem ser calculados depois que o snapshot vivo e o índice
// SQLite representarem a mesma versão confirmada do histórico.
function dadosFinanceirosConfirmados() {
if (exibindoDadosSalvos) return true;
return snapshotAoVivoConfirmado && resumoHistoricoPodeSerPublicado(versaoIndiceHistorico);
}
let consultaHistoricoFechadoGeracao = 0;
let ultimoFetchTs = null;
let autoRefreshAtivo = false;
let autoRefreshTimer = null;
let periodoGraficoAtual = -1;
let periodoGraficoPadraoPendente = true;
let tipoGraficoAtual = "patrimonio";
let casaGraficoAtual = "todas";
let pnlHistoricoDb = null;
let pnlHistoricoDbEmAndamento = null;
let viewAtual = "dashboard";
let trocaViewRenderToken = 0;
// Atualizações de dados continuam acontecendo em todas as abas. Este estado
// indica apenas que a pintura do Dashboard foi adiada enquanto ele estava oculto.
let dashboardVisualPendente = false;
function dashboardEstaVisivel() { return viewAtual === "dashboard"; }
// Estado visual inicial: impede que a toolbar de Posições apareça no Dashboard
// enquanto a página ainda está inicializando os dados.
try { document.body.setAttribute("data-betty-view", "dashboard"); } catch (_) {}
// As caixas só mostram números depois de um snapshot completo confirmado pela API.
// O cache local pode aparecer na tabela durante o carregamento, mas nunca é usado
// como valor final de Dashboard/Estatísticas.
let snapshotAoVivoConfirmado = false;
const SYNC_STATUS_KEY = "betty:syncStatus:v1";
function casaSyncPadrao(casa) {
return { casa, connected: false, lastOk: 0, lastError: null, openCount: 0, closedCount: 0, state: "idle" };
}
let casaSync = Object.fromEntries(CASAS_STAKE.map(casa => [casa, casaSyncPadrao(casa)]));
function lerSyncPersistido() {
try {
const raw = JSON.parse(localStorage.getItem(SYNC_STATUS_KEY) || "null");
if (!raw || typeof raw !== "object") return;
for (const casa of ["polymarket", "kalshi"]) {
if (raw[casa] && typeof raw[casa] === "object") casaSync[casa] = { ...casaSyncPadrao(casa), ...raw[casa] };
}
} catch (_) {}
}
function salvarSyncPersistido() {
try {
const { polymarket, kalshi } = casaSync;
localStorage.setItem(SYNC_STATUS_KEY, JSON.stringify({ polymarket, kalshi }));
} catch (_) {}
}
lerSyncPersistido();
let syncPanelAberto = false;
let BETTY_USER = null;
let BETTY_CONNECTIONS = null;
async function inicializarContaBetty() {
const res = await fetch("/api/me", { cache: "no-store", headers: { "Accept": "application/json" } });
if (!res.ok) { location.href = "/"; throw new Error("Sessão necessária"); }
const data = await res.json();
BETTY_USER = data.user;
BETTY_CONNECTIONS = data.connections || {};
atualizarSaldosFixos();
// O filtro de Estatísticas é individual por usuário. Releia-o somente depois
// de conhecer a conta e deixe o gráfico escolher seu padrão correto: Período
// quando houver uma data salva, All quando não houver.
periodoEstatCarregadoPara = null;
periodoGraficoPadraoPendente = true;
carregarPeriodoEstatisticas();
const h1 = document.getElementById("topbarTitle");
if (h1 && viewAtual === "dashboard") h1.textContent = TITULOS_SECAO.visao[0];
const sub = document.getElementById("topbarSub");
if (sub && viewAtual === "dashboard") sub.textContent = TITULOS_SECAO.visao[1];
const footer = document.querySelector(".sidebar-footer");
if (footer && !document.getElementById("bettyUserBox")) {
const box = document.createElement("div");
box.id = "bettyUserBox";
box.innerHTML = `<span class="sidebar-account-avatar" aria-hidden="true">${escapeHtml(BETTY_USER.name.trim().charAt(0).toUpperCase())}</span><div class="sidebar-account-info"><strong class="sidebar-user-name">${escapeHtml(BETTY_USER.name)}</strong><small>Conta local</small></div><button type="button" class="sidebar-user-logout" aria-label="Sair" data-on-click="logoutBetty()"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M10 5H5v14h5M14 8l4 4-4 4M8 12h10"/></svg></button>`;
footer.append(box);
}
// A configuração passa a ser do usuário logado, não global.
await sincronizarConfiguracaoRemota(false);
if (!casasConectadas().length && !BETTY_CONNECTIONS?.trackedHistory) {
setTimeout(() => irPara("conexoes"), 100);
}
}
async function logoutBetty() {
await fetch("/api/auth/logout", { method: "POST" }).catch(() => {});
location.href = "/";
}
document.addEventListener("DOMContentLoaded", async () => {
posicionarStatusResponsivo();
renderizarConfiguracaoNovasCasas();
await inicializarContaBetty().catch(() => { location.href = "/"; return; });
renderFiltroDataBox();
atualizarFiltroEsportePosicoes();
// Cache local só pode ser exibido para casas atualmente conectadas.
// Isso evita que uma conta recém-criada apareça com apostas antigas do navegador.
// Primeiro usamos o snapshot normal. Se ele estiver vazio (por exemplo,
// porque uma resposta temporária da API foi gravada antes de um reload),
// recuperamos o último snapshot não vazio conhecido.
saldosSalvosDaAbertura = lerSaldosSalvos();
const cacheNormal = lerCacheLocal(true);
const cache = (cacheNormal && ((cacheNormal.abertos || []).length || (cacheNormal.fechados || []).length))
? cacheNormal
: lerUltimoSnapshotNaoVazio();
if (cache) {
const filtroPlataforma = d => Boolean(BETTY_CONNECTIONS?.[casaDaAposta(d)]);
abertos = (cache.abertos || []).filter(filtroPlataforma);
fechados = (cache.fechados || []).filter(filtroPlataforma);
if (abertos.length || fechados.length) {
carregado.aberto = true;
carregado.fechado = true;
ultimoFetchTs = Number(cache.t || 0) || 0;
} else {
limparCachesBetty();
abertos = [];
fechados = [];
}
}
snapshotAoVivoConfirmado = false;
// Sem casas conectadas, a conta começa vazia e não consulta nenhuma API.
if (casasConectadas().length || BETTY_CONNECTIONS?.trackedHistory) {
await abrirComDadosSalvos();
carregarAba("aberto", { forcar: true });
}
else {
liberarNavegacaoInicial();
document.getElementById('syncProgress').hidden = true;
posAtualizarDados();
}
// Atualização automática começa DESATIVADA por padrão.
// Se o usuário já escolheu uma preferência anteriormente, respeitamos essa escolha.
try {
const prefAuto = localStorage.getItem("betty:autoRefresh");
if (prefAuto === "1") autoRefreshAtivo = true;
else if (prefAuto === "0") autoRefreshAtivo = false;
} catch (_) {}
// O timer de atualização automática só existe quando o usuário ativou a função.
// Se estiver desligada, nenhum polling automático é iniciado.
if (autoRefreshAtivo) iniciarAutoRefresh();
atualizarIndicadorAutoRefresh();
atualizarStatusOnline(false);
setInterval(tickAtualizado, 1000);
});
function posicionarStatusResponsivo() {
const media=window.matchMedia('(min-width:901px)');
const mover=()=>{
const status=document.querySelector('.online-wrap');
const destino=document.getElementById(media.matches?'sidebarSyncStatusSlot':'headerSyncStatusSlot');
if (!status || !destino) return;
document.getElementById('headerSyncStatusSlot').hidden=media.matches;
document.getElementById('sidebarSyncStatusSlot').hidden=!media.matches;
if (syncPanelAberto) toggleSyncPanel();
destino.append(status);
};
media.addEventListener('change',mover);
mover();
}
function resetarFiltrosTabela({ renderizar = false } = {}) {
filtroCasaAtual = "todas";
const casaSelect = document.getElementById("filtroCasaSelect");
if (casaSelect) casaSelect.value = "todas";
filtroEsportePosicoesAtual = "todos";
const esporteSelect = document.getElementById("filtroEsporteSelect");
if (esporteSelect) esporteSelect.value = "todos";
filtroDataDia = null;
sortCol = null;
sortDir = "asc";
paginaAtual = 1;
const labelCasa = document.getElementById("casaValueLabel");
if (labelCasa) {
labelCasa.textContent = "Todas";
labelCasa.classList.remove(...CASAS_STAKE.map(casa => "casa-" + casa));
}
const popupCasa = document.getElementById("casaPopup");
if (popupCasa) {
popupCasa.querySelectorAll("button").forEach(b => {
b.classList.toggle("active", b.dataset.value === "todas");
});
}
const labelData = document.getElementById("dataValueLabel");
if (labelData) {
labelData.textContent = "Todas";
labelData.classList.remove("active");
}
fecharPopupCasa();
if (typeof renderFiltroDataBox === "function") renderFiltroDataBox();
if (typeof fecharPopupCalendario === "function") fecharPopupCalendario();
const busca = document.getElementById("busca");
if (busca) busca.value = "";
// select ordenar volta ao padrão
const sel = document.getElementById("ordenar");
if (sel) {
sel.innerHTML = `
<option value="inicio-asc" selected>Data</option>
<option value="negociado">Negociado</option>
`;
}
if (renderizar) aplicarFiltros();
}
function atualizarStatusConexoes() {
for (const casa of CASAS_STAKE) {
const card=document.querySelector('#connectionGrid [data-house="' + casa + '"]');
if (!card) continue;
const connected=Boolean(BETTY_CONNECTIONS?.[casa]);
card.classList.toggle('is-connected',connected);
const status=card.querySelector('.hint');
status.textContent=connected ? 'Conectada' : 'Não conectada';
let detail=card.querySelector('.connection-detail');
if (!detail) {
detail=document.createElement('span');
detail.className='connection-detail';
status.after(detail);
}
detail.textContent=connected ? (casa==='polymarket' ? BETTY_CONNECTIONS.polymarketWalletHint || '' : 'Protegida') : '';
const primary=card.querySelector('.btn-save');
primary.textContent=connected ? 'Alterar' : '+ Conectar';
if (!card.querySelector('[data-connection-test]')) {
const test=document.createElement('button');
test.type='button'; test.className='btn-ghost';
test.dataset.connectionTest=casa;
test.textContent='Testar';
test.onclick=()=>testarConexaoCasa(casa);
card.querySelector('.connection-actions').insertBefore(test,primary.nextSibling);
}
for (const button of card.querySelectorAll('.btn-ghost')) {
const test=button.dataset.connectionTest===casa;
button.disabled=!connected;
button.textContent=test ? 'Testar' : '×';
if (!test) button.setAttribute('aria-label','Desconectar ' + NOMES_CASAS[casa]);
}
}
}
function renderizarConfiguracaoNovasCasas() {
for (const [casa, nome] of Object.entries(CASAS_NOVAS)) {
document.getElementById("connectionGrid").insertAdjacentHTML("beforeend", `
<section class="stake-platform-card connection-card" data-house="${casa}">
<h3><img src="/assets/platforms/${casa}.png" alt="" width="28" height="28">${nome}</h3>
<p class="hint" id="${casa}ConnectionStatus">Não conectada</p>
<div class="connection-actions">
<button type="button" class="btn-save" data-on-click="abrirContaModal('${casa}')">Conectar / alterar</button>
<button type="button" class="btn-ghost" id="${casa}Disconnect" data-on-click="desconectarNovaCasa('${casa}')" disabled>Desconectar</button>
</div>
</section>`);
document.getElementById("closedRangeGrid").insertAdjacentHTML("beforeend", `
<section class="stake-platform-card closed-range-card">
<h3>${nome}</h3>
<div class="closed-range-fields">
${[["Start", "start", "Data inicial"], ["End", "end", "Data final"]].map(([campo, tipo, label]) => `
<div class="config-field">
<label for="cfgClosedRange${campo}-${casa}Button">${label}</label>
<input type="hidden" id="cfgClosedRange${campo}-${casa}">
<button type="button" class="config-date-button" id="cfgClosedRange${campo}-${casa}Button"
  data-on-click="toggleCalendarioPeriodoFechadasConfigurado('${casa}','${tipo}',event)" aria-haspopup="dialog" aria-expanded="false">
<span id="cfgClosedRange${campo}-${casa}Value">dd/mm/aaaa</span>
</button>
</div>`).join("")}
<button type="button" class="closed-range-clear" data-on-click="limparPeriodoFechadasConfigurado('${casa}')" aria-label="Limpar período de ${nome}">×</button>
</div>
</section>`);
document.getElementById("stakeGrid").insertAdjacentHTML("beforeend", `
<section class="stake-platform-card">
<h3>${nome}</h3>
<div class="config-field">
<label for="cfgStakeUnidade-${casa}">Stake padrão — 1 u ($)</label>
<input type="number" id="cfgStakeUnidade-${casa}" step="0.01" min="0.01" inputmode="decimal">
</div>
<label style="display:block;font-size:.72rem;color:var(--muted);margin:4px 0 8px;">Mudanças por data</label>
<div class="stake-rules" id="stakeRulesList-${casa}"></div>
<button type="button" class="btn-add-rule" data-on-click="adicionarRegraStake('${casa}')">+ Adicionar mudança</button>
</section>`);
}
organizarConfiguracaoPorCasa();
}
function organizarConfiguracaoPorCasa() {
const ranges=document.getElementById('closedRangeGrid'),stakes=document.getElementById('stakeGrid');
if (!ranges || !stakes) return;
const section=ranges.closest('.config-card'),oldStake=stakes.closest('.config-card');
section.id='houseSettingsCard';
section.querySelector('h2').innerHTML='<svg class="connections-heading-icon" aria-hidden="true" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"><path d="M3 6h18M3 12h18M3 18h18"/><circle cx="8" cy="6" r="2" fill="var(--bg)"/><circle cx="16" cy="12" r="2" fill="var(--bg)"/><circle cx="10" cy="18" r="2" fill="var(--bg)"/></svg>Período por casa';
section.querySelector('.hint').textContent='Datas de venda ou resolução. Sem datas, todo o histórico.';
const scroll=document.createElement('div');
scroll.className='config-matrix-scroll';
scroll.tabIndex=0;
scroll.setAttribute('aria-label','Período por casa');
const table=document.createElement('table');
table.className='config-matrix';
table.innerHTML='<thead><tr><th scope="col"><span class="sr-only">Configuração</span></th>'+
CASAS_STAKE.map(casa=>'<th scope="col"><span class="config-house-name"><img src="/assets/platforms/'+casa+'.png" alt="" width="24" height="24">'+escapeHtml(NOMES_CASAS[casa])+'</span></th>').join('')+
'</tr></thead><tbody></tbody>';
const body=table.querySelector('tbody');
const row=label=>{
const tr=document.createElement('tr'),th=document.createElement('th');
th.scope='row';th.textContent=label;tr.append(th);body.append(tr);
return tr;
};
for (const [label,field] of [['Data inicial','Start'],['Data final','End']]) {
const tr=row(label);
for (const casa of CASAS_STAKE) {
const td=document.createElement('td');
const button=document.getElementById('cfgClosedRange'+field+'-'+casa+'Button');
td.append(button.closest('.config-field'));
if (field==='End') {
const clear=ranges.querySelector('[onclick="limparPeriodoFechadasConfigurado(\''+casa+'\')"]');
if (clear) td.append(clear);
}
tr.append(td);
}
}
const units=row('1 u padrão (US$)'),rules=row('Mudanças por data');
for (const casa of CASAS_STAKE) {
const unitCell=document.createElement('td');
unitCell.append(document.getElementById('cfgStakeUnidade-'+casa).closest('.config-field'));
units.append(unitCell);
const ruleCell=document.createElement('td'),list=document.getElementById('stakeRulesList-'+casa);
const add=list.parentElement.querySelector('.btn-add-rule');
add.textContent='+ Regra';
ruleCell.append(list,add);rules.append(ruleCell);
}
scroll.append(table);
ranges.replaceWith(scroll);
const stakeScroll=document.createElement('div');
stakeScroll.className='config-matrix-scroll';
stakeScroll.tabIndex=0;
stakeScroll.setAttribute('aria-label','Stake por casa');
const stakeTable=document.createElement('table');
stakeTable.className='config-matrix';
stakeTable.append(table.querySelector('thead').cloneNode(true));
const stakeBody=document.createElement('tbody');
stakeBody.append(units,rules);
stakeTable.append(stakeBody);
stakeScroll.append(stakeTable);
oldStake.id='stakeSettingsCard';
oldStake.querySelector('h2').innerHTML=section.querySelector('h2').innerHTML.replace('Período por casa','Stake por casa');
oldStake.querySelector('.hint').textContent='Defina o valor de 1 u e as mudanças por data para cada casa.';
stakes.replaceWith(stakeScroll);
for (const [card,content] of [[section,scroll],[oldStake,stakeScroll]]) {
const heading=card.querySelector('h2'),toggle=document.createElement('button');
content.id=card.id+'Content';
toggle.type='button';toggle.className='config-section-toggle';
const preferenceKey='betty:configCollapsed:'+card.id;
try {content.hidden=localStorage.getItem(preferenceKey)==='1';}catch(_){}
toggle.setAttribute('aria-expanded',String(!content.hidden));toggle.setAttribute('aria-controls',content.id);
toggle.append(...heading.childNodes);
toggle.insertAdjacentHTML('beforeend','<svg class="config-section-chevron" aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="m6 9 6 6 6-6"/></svg>');
const hint=card.querySelector('.hint'),description=document.createElement('span');
description.className='config-section-description';description.textContent=hint.textContent;
toggle.append(description);hint.remove();
toggle.addEventListener('click',()=>{
content.hidden=!content.hidden;
toggle.setAttribute('aria-expanded',String(!content.hidden));
try {localStorage.setItem(preferenceKey,content.hidden?'1':'0');}catch(_){}
if(content.hidden && card===section)fecharCalendarioPeriodoFechadasConfigurado();
});
heading.append(toggle);
}
}
let contaModalTipo = null;
let contaModalFoco = null;
let polyPreview = null;
function abrirContaModal(tipo){
contaModalTipo=tipo; polyPreview=null;
contaModalFoco = document.activeElement;
const modal=document.getElementById("accountModal"), fields=document.getElementById("accountModalFields");
const nome = CASAS_NOVAS[tipo];
if (nome) {
const predict = tipo === "predict";
document.getElementById("accountModalTitle").textContent = "Conectar " + nome;
document.getElementById("accountModalSub").textContent = "A chave de API fica criptografada no Betty.";
fields.innerHTML = `
${predict ? '<label for="novaCasaWallet">Endereço público da conta Predict.fun</label><input id="novaCasaWallet" placeholder="0x..." autocomplete="off" spellcheck="false">' : ''}
<label for="novaCasaApiKey">Chave de API</label><input id="novaCasaApiKey" type="password" autocomplete="off">
<div class="connection-note">${predict
? 'Use o endereço da carteira da própria conta Predict.fun e uma chave do <a href="https://developers.predict.fun/" target="_blank" rel="noopener">console de desenvolvedores</a>.'
: 'Use uma chave da seção Conta da SX Bet com acesso de leitura aos dados da sua conta.'}</div>`;
document.getElementById("accountModalError").textContent = "";
document.getElementById("accountModalSave").textContent = "Salvar conexão";
modal.classList.add("show");
modal.setAttribute("aria-hidden", "false");
fields.querySelector("input").focus();
return;
}
document.getElementById("accountModalTitle").textContent=tipo==="poly"?"Conectar Polymarket":"Conectar Kalshi";
document.getElementById("accountModalSub").textContent=tipo==="poly"?"Informe a wallet pública. Validamos a proxy (onde ficam fundos e posições) antes de confirmar.":"As credenciais são enviadas por HTTPS e armazenadas criptografadas no servidor. Nunca ficam no HTML.";
fields.innerHTML=tipo==="poly"?`<label>Wallet pública</label><input id="modalWallet" placeholder="0x..." autocomplete="off"><div id="polyPreviewBox"></div><div class="connection-note">Cole o endereço de login ou a proxy wallet. O Betty resolve a proxy e mostra saldo e posições antes de gravar. Nunca envie seed phrase nem Private Key.</div>`:`<label>API Key ID (Read only)</label><input id="modalKalshiKey" placeholder="Ex.: 123e4567-e89b-12d3-a456-426614174000" autocomplete="off"><label>Private Key PEM</label><textarea id="modalKalshiPrivate" placeholder="-----BEGIN PRIVATE KEY-----
...
-----END PRIVATE KEY-----"></textarea><div class="connection-note">Use uma API Key <strong>Read only</strong> da Kalshi. A API Key ID e a Private Key são enviadas somente ao backend e ficam criptografadas. O Betty não envia ordens nem precisa de permissão de escrita.</div>`;
document.getElementById("accountModalError").textContent="";
const save=document.getElementById("accountModalSave");
if (save) save.textContent = tipo==="poly" ? "Validar carteira" : "Salvar conexão";
modal.classList.add("show"); modal.setAttribute("aria-hidden","false");
if (tipo==="poly") {
const inp=document.getElementById("modalWallet");
if (inp) inp.addEventListener("input", () => {
if (!polyPreview) return;
polyPreview=null;
const box=document.getElementById("polyPreviewBox");
if (box) box.innerHTML="";
if (save) save.textContent="Validar carteira";
});
}
}
function fecharContaModal(){
const m = document.getElementById("accountModal");
m.classList.remove("show");
m.setAttribute("aria-hidden", "true");
document.getElementById("accountModalFields").replaceChildren();
contaModalTipo = null;
polyPreview = null;
contaModalFoco?.focus();
contaModalFoco = null;
}
document.getElementById("accountModal").addEventListener("keydown", event => {
if (event.key === "Escape") fecharContaModal();
if (event.key === "Enter" && event.target.matches("input") && Object.hasOwn(CASAS_NOVAS, contaModalTipo)) {
event.preventDefault();
document.getElementById("accountModalSave").click();
}
});
async function atualizarConexaoNovaCasa(casa, method, credentials) {
const response = await fetch("/api/connections/" + casa, {
method, headers: { "Content-Type": "application/json" },
body: credentials ? JSON.stringify(credentials) : undefined
});
const data = await response.json();
if (!response.ok || !data.ok) throw new Error(data.error || "Não foi possível atualizar a conexão.");
BETTY_CONNECTIONS = data.connections;
if (casa === "predict") {
PREDICT_API_BALANCE = null;
saldoPredictFalhou = false;
}
if (casa === "sxbet") SXBET_API_BALANCE = null;
atualizarSaldosFixos();
casaSync[casa] = casaSyncPadrao(casa);
syncExportAt[casa] = "";
abertos = abertos.filter(d => casaDaAposta(d) !== casa);
fechados = fechados.filter(d => casaDaAposta(d) !== casa);
snapshotAoVivoConfirmado = false;
const versaoIndice = iniciarSincronizacaoIndiceHistorico();
limparCachesBetty();
limparCachesPeriodoFechadas();
for (const id of ["stats", "dashboardInsights", "chartArea", "chartDateNotice", "statsEstat", "statsExtraMetrics", "statsSubContent", "content"]) {
document.getElementById(id)?.replaceChildren();
}
atualizarStatusConexoes();
if (contaModalTipo === casa) fecharContaModal();
await carregarAba(abaAtual, { forcar: true, versaoIndice });
}
async function desconectarNovaCasa(casa) {
const button = document.getElementById(casa + "Disconnect");
button.disabled = true;
try { await atualizarConexaoNovaCasa(casa, "DELETE"); }
catch (erro) { alert(erro.message); atualizarStatusConexoes(); }
}
function renderPolyPreview(d) {
const box=document.getElementById("polyPreviewBox");
if (!box) return;
const abertas = d.openPositionsCapped ? d.openPositions + "+" : String(d.openPositions);
box.innerHTML = `<div class="preview-box">
<div class="row"><span>Proxy wallet</span><strong>${escapeHtml(d.proxyWalletHint || "")}</strong></div>
${d.resolvedFromLogin ? `<div class="row"><span>Endereço informado</span><span>login — resolvido para a proxy</span></div>` : ""}
<div class="row"><span>Saldo pUSD</span><strong>$${Number(d.balance||0).toFixed(2)}</strong></div>
<div class="row"><span>Posições abertas</span><strong>${abertas}</strong></div>
${d.warning ? `<div class="warn">${escapeHtml(d.warning)}</div>` : `<div class="muted" style="margin-top:8px;color:var(--muted);font-size:.74rem">Confira se o saldo e as posições batem com a conta na Polymarket antes de confirmar.</div>`}
</div>`;
}
document.getElementById("accountModalSave").onclick=async()=>{
const err=document.getElementById("accountModalError"),btn=document.getElementById("accountModalSave"); btn.disabled=true; err.textContent="";
try{
if (Object.hasOwn(CASAS_NOVAS, contaModalTipo)) {
await atualizarConexaoNovaCasa(contaModalTipo, "POST", {
apiKey: document.getElementById("novaCasaApiKey").value.trim(),
wallet: document.getElementById("novaCasaWallet")?.value.trim()
});
} else if(contaModalTipo==="poly"){
const wallet=document.getElementById("modalWallet").value.trim();
if(!polyPreview){
const r=await fetch("/api/connections/polymarket/preview",{method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify({wallet})});
const d=await r.json();
if(!r.ok || !d.ok) throw new Error(d.error||"Não foi possível validar a wallet.");
polyPreview=d;
renderPolyPreview(d);
btn.textContent="Confirmar conta";
return;
}
const r=await fetch("/api/connections/polymarket",{method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify({wallet})});
const d=await r.json(); if(!r.ok)throw new Error(d.error||"Não foi possível salvar a conexão.");
BETTY_CONNECTIONS=d.connections; KALSHI_API_BALANCE=null; POLY_API_BALANCE=null; atualizarStatusConexoes(); limparCachesBetty(); abertos=[]; fechados=[]; snapshotAoVivoConfirmado=false; carregado={aberto:false,fechado:false}; fecharContaModal(); await atualizarAgora();
}else{
const r=await fetch("/api/connections/kalshi",{method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify({apiKey:document.getElementById("modalKalshiKey").value.trim(),privateKey:document.getElementById("modalKalshiPrivate").value.trim()})});
const d=await r.json(); if(!r.ok)throw new Error(d.error||"Não foi possível salvar a conexão.");
BETTY_CONNECTIONS=d.connections; KALSHI_API_BALANCE=null; POLY_API_BALANCE=null; atualizarStatusConexoes(); limparCachesBetty(); abertos=[]; fechados=[]; snapshotAoVivoConfirmado=false; carregado={aberto:false,fechado:false}; fecharContaModal(); await atualizarAgora();
}
}catch(e){err.textContent=e.message}finally{btn.disabled=false}
};
async function conectarPolymarket(){abrirContaModal("poly")}
async function desconectarPolymarket() {
if(!confirm("Desconectar a Polymarket desta conta?")) return;
const r=await fetch("/api/connections/polymarket",{method:"DELETE"}); const d=await r.json();
if(r.ok){BETTY_CONNECTIONS=d.connections;POLY_API_BALANCE=null;atualizarStatusConexoes();atualizarSaldosFixos();limparCachesBetty();abertos=[];fechados=[];snapshotAoVivoConfirmado=false;carregado={aberto:false,fechado:false};posAtualizarDados()}
}
async function fetchPolymarketSaldo() {
if (!BETTY_CONNECTIONS?.polymarket) return null;
const res = await fetch("/api/polymarket/balance", { cache: "no-store" });
if (!res.ok) throw new Error("Polymarket balance HTTP " + res.status);
const data = await res.json();
if (!data.ok) throw new Error(data.error || "Polymarket balance error");
const balance = Number(data.balance);
if (!Number.isFinite(balance)) throw new Error("Saldo da Polymarket inválido.");
return balance;
}
async function fetchKalshiSaldo() {
if (!BETTY_CONNECTIONS?.kalshi) return null;
const res = await fetchComRetry(KALSHI_PROXY + "/balance", { cache: "no-store" });
if (!res.ok) throw new Error("Kalshi balance HTTP " + res.status);
const data = await res.json();
if (!data.ok) throw new Error(data.error || "Kalshi balance error");
const balance = Number(data.balance);
if (!Number.isFinite(balance)) throw new Error("Saldo da Kalshi inválido.");
return balance;
}
async function fetchPredictSaldo() {
const response = await fetch("/api/tracked/predict/balance", { cache: "no-store" });
if (!response.ok) throw new Error("Saldo do Predict.fun indisponível.");
const data = await response.json();
if (!data.ok || !Number.isFinite(data.balance) || data.balance < 0) throw new Error("Saldo do Predict.fun inválido.");
return data.balance;
}
async function fetchSxBetSaldo() {
const response = await fetch("/api/tracked/sxbet/balance", { cache: "no-store" });
const data = await response.json().catch(() => null);
if (!response.ok || !data?.ok || !Number.isFinite(data.balance) || !Number.isFinite(data.escrowedAmount)) {
throw new Error(data?.error || "Não foi possível consultar o saldo da SX Bet.");
}
return data;
}
async function atualizarSaldoKalshiAPI() {
if (!BETTY_CONNECTIONS?.kalshi) {
KALSHI_API_BALANCE = null;
atualizarSaldosFixos();
return;
}
try {
const balance = await fetchKalshiSaldo();
if (Number.isFinite(balance)) {
KALSHI_API_BALANCE = balance;
atualizarSaldosFixos();
}
} catch (e) {
// Em caso de falha temporária, preserva o último saldo confirmado em vez de
// voltar para o cálculo por depósito ou mostrar um valor incorreto.
console.warn("Saldo Kalshi não atualizado:", e);
}
}
async function conectarKalshi(){abrirContaModal("kalshi")}
let testeConexaoVersao=0;
let testeConexaoFoco=null;
function fecharTesteConexao() {
testeConexaoVersao++;
const modal=document.getElementById('connectionTestModal');
modal.classList.remove('show');
modal.setAttribute('aria-hidden','true');
testeConexaoFoco?.focus();
}
async function testarKalshi(){ return testarConexaoCasa('kalshi'); }
async function testarConexaoCasa(casa) {
if (!BETTY_CONNECTIONS?.[casa]) return;
const versao=++testeConexaoVersao;
const conta=BETTY_USER?.id;
const conexoes=BETTY_CONNECTIONS;
testeConexaoFoco=document.activeElement;
const modal=document.getElementById('connectionTestModal');
const status=document.getElementById('connectionTestStatus');
const values=document.getElementById('connectionTestValues');
document.getElementById('connectionTestTitle').textContent='Conexão · '+NOMES_CASAS[casa];
status.textContent='Testando conexão…';
status.className='muted';
values.replaceChildren();
modal.classList.add('show');
modal.setAttribute('aria-hidden','false');
modal.querySelector('.account-close').focus();
try {
let balance=null,portfolio=null;
let portfolioLabel='Valor do portfólio';
let sxBetBalanceError='';
if (casa==='kalshi') {
const response=await fetchComRetry('/api/kalshi/test',{cache:'no-store'});
const data=await response.json();
if (!response.ok || !data.ok) throw new Error(data.error || 'Não foi possível consultar a Kalshi.');
balance=data.balance; portfolio=data.portfolioValue;
} else if (casa==='polymarket') {
balance=await fetchPolymarketSaldo();
} else {
const [response,sxBetSaldo]=await Promise.all([
fetch('/api/connections/'+casa+'/test',{cache:'no-store'}),
casa==='sxbet' ? fetchSxBetSaldo().catch(error=>{
sxBetBalanceError=error instanceof TypeError ? 'Não foi possível consultar o saldo da SX Bet.' : error.message;
return null;
}) : null
]);
const data=await response.json();
if (!response.ok || !data.ok) throw new Error(data.error || 'Não foi possível validar a conexão.');
if (casa==='predict') balance=await fetchPredictSaldo();
if (sxBetSaldo) {
balance=sxBetSaldo.balance;
portfolio=sxBetSaldo.escrowedAmount;
portfolioLabel='Em garantia nas apostas';
}
}
if (versao!==testeConexaoVersao || conta!==BETTY_USER?.id || conexoes!==BETTY_CONNECTIONS) return;
if (sxBetBalanceError) {
status.textContent='Conexão confirmada. '+sxBetBalanceError;
return;
}
status.textContent='Conexão confirmada';
status.className='connection-test-success';
const money=value=>typeof value==='number' && Number.isFinite(value)
? value.toLocaleString('pt-BR',{style:'currency',currency:'USD'}) : 'Não informado pela plataforma';
for (const [label,value] of [['Saldo disponível',balance],[portfolioLabel,portfolio]]) {
const term=document.createElement('dt'),detail=document.createElement('dd');
term.textContent=label; detail.textContent=money(value);
values.append(term,detail);
}
} catch(error) {
if (versao!==testeConexaoVersao || conta!==BETTY_USER?.id || conexoes!==BETTY_CONNECTIONS) return;
status.textContent=error.message || 'Não foi possível concluir o teste.';
status.className='account-error';
}
}
document.getElementById('connectionTestModal').addEventListener('click',event=>{
if (event.target===event.currentTarget) fecharTesteConexao();
});
document.getElementById('connectionTestModal').addEventListener('keydown',event=>{
if (event.key==='Escape') { event.preventDefault(); event.stopPropagation(); fecharTesteConexao(); }
if (event.key==='Tab') {
const buttons=[...event.currentTarget.querySelectorAll('button')];
if (event.shiftKey && document.activeElement===buttons[0]) {event.preventDefault();buttons.at(-1).focus();}
else if (!event.shiftKey && document.activeElement===buttons.at(-1)) {event.preventDefault();buttons[0].focus();}
}
});
async function desconectarKalshi() {
if(!confirm("Desconectar a Kalshi desta conta?")) return;
const r=await fetch("/api/connections/kalshi",{method:"DELETE"}); const d=await r.json();
if(r.ok){
BETTY_CONNECTIONS=d.connections;
KALSHI_API_BALANCE = null;
abertos=abertos.filter(d => (d?._platform||"").toLowerCase() !== "kalshi");
fechados=fechados.filter(d => (d?._platform||"").toLowerCase() !== "kalshi");
atualizarStatusConexoes();
limparCachesBetty();
salvarCacheLocal();
snapshotAoVivoConfirmado = !(casasConectadas().length);
if (casasConectadas().length) {
carregarAba("aberto", { forcar: true });
} else {
posAtualizarDados();
}
}
}
function limparCachesBetty(){
// Conta ou conexão mudou: o estado salvo deixa de representar o que está conectado.
encerrarDadosSalvos();
try { localStorage.removeItem(SALDOS_SALVOS_KEY); } catch (_) {}
paginasHistoricoPosicoes.clear();
escopoPaginasHistoricoPosicoes = '';
try{
const userId=BETTY_USER?.id||"local";
localStorage.removeItem(CACHE_KEY);localStorage.removeItem(LAST_NONEMPTY_CACHE_KEY);
for(const k of (LEGACY_CACHE_KEYS||[])) localStorage.removeItem(k);
localStorage.removeItem(DATE_CACHE_KEY);localStorage.removeItem(GAMMA_DATE_META_CACHE_KEY);localStorage.removeItem(SPORT_META_CACHE_KEY);localStorage.removeItem(POLY_ACTIVITY_AT_KEY);
localStorage.removeItem(POLY_CLOSED_ENRICH_CACHE_PREFIX+userId);
localStorage.removeItem('betty:lastConfirmedClosed:'+userId);
}catch(e){}
try { polyActivityCache={at:0,conditionIds:[]}; } catch (_) {}
try { polyFluxoCache={at:0,porMercado:null}; } catch (_) {}
try { polyClosedSnapshotCache={at:0,data:[]}; } catch (_) {}
try { kalshiClosedSnapshotCache={at:0,data:[]}; } catch (_) {}
try { polyClosedEnrichmentCache=null; } catch (_) {}
}
function trocarView(view) {      if (document.body.classList.contains('initial-loading')) return;
fecharExportPopup();
const viewAnterior = viewAtual;
if (viewAtual === 'estatisticas' && view !== 'estatisticas') {
subAbaEstat = 'site';
document.querySelectorAll('.stat-subtab').forEach(button => button.classList.toggle('active',button.dataset.sub === 'site'));
}
viewAtual = view;
const renderToken = ++trocaViewRenderToken;
// Atualiza o estado visual antes de qualquer renderização/consulta.
// Isso impede a toolbar de Posições de aparecer no Dashboard durante refreshs.
try { document.body.setAttribute("data-betty-view", view); } catch (_) {}
// Os filtros pertencem à tela de posições. Não é necessário resetá-los ao
// alternar entre as outras telas, nem renderizar uma tabela que ficará oculta.
if (viewAnterior !== view && (viewAnterior === "posicoes" || view === "posicoes")) resetarFiltrosTabela();
const isTabela = view === "posicoes";
const isEstat = view === "estatisticas";
const isConfig = view === "config";
const isCalendario = view === "calendario";
const stats = document.getElementById("stats");
const chartSection = document.getElementById("chartSection");
const toolbar = document.getElementById("toolbarSection");
const tableSection = document.getElementById("tableSection");
const statsView = document.getElementById("statsView");
const configView = document.getElementById("configView");
const h1 = document.getElementById("topbarTitle");
const sub = document.getElementById("topbarSub");
if (stats) stats.style.display = (isTabela || isEstat || isConfig || isCalendario) ? "none" : "";
if (chartSection) chartSection.style.display = (isTabela || isEstat || isConfig || isCalendario) ? "none" : "";
const insights = document.getElementById("dashboardInsights");
if (insights) insights.style.display = (isTabela || isEstat || isConfig || isCalendario) ? "none" : "";
document.getElementById("profitCalendarView").style.display = isCalendario ? "" : "none";
if (toolbar) {
// Esta barra pertence EXCLUSIVAMENTE a Posições.
// Em Estatísticas ela deve desaparecer por completo.
toolbar.style.display = isTabela ? "" : "none";
toolbar.classList.toggle("stats-hidden", !isTabela);
toolbar.classList.remove("toolbar-stats-mode");
toolbar.classList.toggle("posicoes-mode", view === "posicoes");
const liveTab = document.getElementById("tabAoVivo");
if (liveTab) liveTab.style.display = view === "posicoes" ? "flex" : "none";
}
if (tableSection) tableSection.style.display = isTabela ? "" : "none";
if (statsView) statsView.style.display = isEstat ? "" : "none";
document.body.classList.toggle("view-estatisticas", isEstat);
document.body.classList.toggle("view-dashboard", view === "dashboard");
if (configView) configView.style.display = isConfig ? "" : "none";
paginaAtual = 1;
if (view !== "posicoes") pararAtualizacaoAoVivo();
if (view === "posicoes" && abaAtual === "ao-vivo") {
abaAtual = "aberto";
document.querySelectorAll(".tabs .tab").forEach(t => t.classList.toggle("active", t.dataset.tab === "aberto"));
}
if (view === "posicoes") {
if (h1) h1.textContent = "Posições";
if (sub) sub.textContent = "Todas as suas posições, ativas e fechadas";
} else if (view === "estatisticas") {
if (h1) h1.textContent = "Estatísticas";
if (sub) sub.textContent = "Análise completa do seu desempenho (posições fechadas)";
} else if (isCalendario) {
if (h1) h1.textContent = "Calendário";
if (sub) sub.textContent = "Seu resultado dia a dia, mês a mês";
} else if (view === "config") {
if (h1) h1.textContent = "Configurações";
if (sub) sub.textContent = "Contas, histórico e preferências de stake";
} else {
if (h1) h1.textContent = TITULOS_SECAO.visao[0];
if (sub) sub.textContent = TITULOS_SECAO.visao[1];
}
atualizarSecaoNavegacao(view);
// Permite que o navegador apresente a aba selecionada antes de executar os
// renderizadores mais custosos. O token ignora renderizações de cliques antigos.
setTimeout(() => {
if (renderToken !== trocaViewRenderToken || viewAtual !== view) return;
if (view === "posicoes") {
voltarOrdemPadrao();
aplicarFiltros();
} else if (view === "estatisticas") {
renderEstatisticas();
} else if (view === "calendario") {
renderProfitCalendar();
} else if (view === "config") {
preencherFormConfiguracao();
atualizarStatusConexoes();
const msg = document.getElementById("configMsg");
if (msg) { msg.textContent = ""; msg.className = "config-msg"; }
} else {
voltarOrdemPadrao();
renderizarStats();
desenharGrafico();
}
}, 0);
}
// Volta a ordenação interna padrão por Data (Dashboard / Posições)
function voltarOrdemPadrao() {
const sel = document.getElementById("ordenar");
if (!sel) return;
if (sel.value !== "inicio-asc") {
sel.innerHTML = `<option value="inicio-asc" selected>Data</option>`;
sortCol = null;
aplicarFiltros();
}
}
function pararAtualizacaoAoVivo() {
if (window.__bettyLiveTimer) {
clearInterval(window.__bettyLiveTimer);
window.__bettyLiveTimer = null;
}
}
function iniciarAtualizacaoAoVivo() {
pararAtualizacaoAoVivo();
window.__bettyLiveTimer = setInterval(() => {
if (viewAtual === "posicoes" && abaAtual === "ao-vivo") renderAoVivo({ manterPagina: true });
else pararAtualizacaoAoVivo();
}, 30000);
}
function trocarAba(aba) {
if (aba === "ao-vivo") {
if (viewAtual !== "posicoes") return;
abaAtual = "ao-vivo";
document.querySelectorAll(".tabs .tab").forEach(t => {
t.classList.toggle("active", t.dataset.tab === "ao-vivo");
});
renderAoVivo();
iniciarAtualizacaoAoVivo();
return;
}
pararAtualizacaoAoVivo();
if (abaAtual === aba) return;
abaAtual = aba;
sortCol = null;
paginaAtual = 1;
document.querySelectorAll(".tabs .tab").forEach(t => {
t.classList.toggle("active", t.dataset.tab === aba);
});
const sel = document.getElementById("ordenar");
if (sel) sel.innerHTML = `<option value="inicio-asc" selected>Data</option>`;
if (!carregado[aba]) {
carregarAba(aba);
} else {
aplicarFiltros();
}
}
function renderAoVivo(opcoes = {}) {
const agora = Date.now();
const duracao = 2 * 60 * 60 * 1000;
// Ao vivo é apenas uma visualização temporária.
// Não altera abertos/fechados, filtros, ledger ou dados usados pelo CSV.
let base = filtrarExcluidos([...(abertos || []), ...(fechados || [])]);
const termo = document.getElementById("filtroTitulo")?.value.toLowerCase().trim() || "";
if (termo) base = base.filter(d => tituloExibicao(d).toLowerCase().includes(termo));
base = filtrarPorPlataforma(base);
base = filtrarPorEsporte(base);
if (filtroDataDia) base = base.filter(d => apostaNoDia(d, filtroDataDia));
const listaAoVivo = base.filter(d => {
const inicio = dataInicioAposta(d);
if (!inicio) return false;
const inicioMs = inicio.getTime();
return Number.isFinite(inicioMs) && inicioMs <= agora && agora < inicioMs + duracao;
});
// Mostra exatamente a mesma tabela normal de Posições.
// Não agrupa por evento/mercado: cada aposta permanece como uma linha.
listaAoVivo.sort((a, b) => compararLinhas(a, b));
listaFiltradaAtual = listaAoVivo;
if (!opcoes.manterPagina) paginaAtual = 1;
renderizarPagina();
// O rodapé normal da tabela continua sendo usado; nada é criado no CSV.
}
// Polymarket: perdas resolvidas ficam em /positions (curPrice≈0)
// e NÃO entram em /closed-positions (só vitórias resgatadas).
// Link correto Kalshi: /markets/{series}/{event_ticker}
function linkKalshi(d) {
let market = String(d.slug || "").replace(/^kalshi:/i, "").trim();
let event = String(d.eventSlug || "").replace(/^kalshi:/i, "").trim();
if (!event && market) {
const parts = market.split("-");
// remove sufixo de outcome (-1, -2) se for só número
if (parts.length >= 2 && /^\d+$/.test(parts[parts.length - 1])) {
event = parts.slice(0, -1).join("-");
} else {
event = market;
}
}
if (!event) return "https://kalshi.com/markets";
let series = String(d.seriesTicker || "").trim();
if (!series) {
const m = event.match(/^(KX[A-Z0-9]+)/i);
series = m ? m[1] : "";
}
series = series.toLowerCase();
event = event.toLowerCase();
if (series) return "https://kalshi.com/markets/" + encodeURIComponent(series) + "/" + encodeURIComponent(event);
return "https://kalshi.com/markets/" + encodeURIComponent(event);
}
function isPerdaResolvida(d) {
return BettyAudit.resolvedLoss(d);
}
function perdaComoFechada(d, i) {
const stake = (d.initialValue != null && d.initialValue > 0)
? d.initialValue
: (d.size != null && d.avgPrice != null ? d.size * d.avgPrice : 0);
// Perda total = −stake (ou cashPnl da API, que deve ser ≈ −stake)
const pnl = d.cashPnl != null ? d.cashPnl : -stake;
// Perdas não têm resgate na API — usa início do jogo + 3h como proxy de "fechamento"
let ts = d.timestamp || 0;
if (!ts) {
const raw = d._startDate || d.endDate;
if (raw) {
const dt = parseApiDate(raw);
if (dt) ts = Math.floor((dt.getTime() + 3 * 60 * 60 * 1000) / 1000);
}
}
return {
...d,
realizedPnl: pnl,
timestamp: ts,
_order: i,
_platform: d._platform || "polymarket",
_perdaResolvida: true
};
}
// Controle de refresh: cada ciclo tem um ID. Uma resposta atrasada nunca pode
// sobrescrever dados de um ciclo mais novo.
let carregandoAba = false;
let refreshGeneration = 0;
// Reconciliação Polymarket: activity é usada somente como fonte de conferência/recuperação.
// Nunca cria apostas diretamente, evitando duplicações e efeitos colaterais.
// O timestamp do último check fica salvo no localStorage (não só em memória)
// para que o intervalo de ~10 min seja respeitado mesmo depois de um F5
// completo (o botão "Atualizar agora" recarrega a página de verdade).
const POLY_ACTIVITY_AT_KEY = "betty:polyActivityAt";
let polyActivityCache = { at: 0, conditionIds: [] };
try {
const savedAt = Number(localStorage.getItem(POLY_ACTIVITY_AT_KEY));
if (Number.isFinite(savedAt) && savedAt > 0) polyActivityCache.at = savedAt;
} catch (_) {}
const POLY_ACTIVITY_CACHE_MS = 10 * 60 * 1000;
let polyClosedSnapshotCache = { at: 0, data: [] };
let kalshiClosedSnapshotCache = { at: 0, data: [] };
const POLY_CLOSED_ENRICH_CACHE_PREFIX = "betty:polyClosedEnriched:";
const POLY_INCOMPLETE_RETRY_MS = 10 * 60 * 1000;
// Incrementa somente quando a regra central de data ganhar uma correção que
// precise revalidar entradas antigas já enriquecidas. Não é uma migração
// recorrente: cada entrada suspeita recebe esta versão após ser conferida.
const POLY_CLOSED_DATE_RESOLVER_VERSION = 1;
let polyClosedEnrichmentCache = null;
const CLOSED_REFRESH_CACHE_MS = 0;
function chaveLedgerFechada(d) {
const plat = (d?._platform || 'polymarket').toLowerCase();
if (plat === 'predict' || plat === 'sxbet') return plat + '|' + String(d?.slug || d?.id || '');
if (plat === 'kalshi') {
// O ticker é a identidade estável do mercado. Não usamos conditionId,
// porque versões anteriores usavam prefixos diferentes para a mesma aposta.
return 'kalshi|' + String(d?.slug || d?.ticker || d?.market_ticker || '').trim();
}
return 'polymarket|' + [d?.conditionId || '', d?.asset || '', d?.outcomeIndex ?? d?.outcome ?? ''].join('|');
}
function tituloFracoLedgerFrontend(v) {
const s = String(v || '').trim();
if (!s) return true;
// Tickers da Kalshi nunca são um nome válido para exibição.
// Eles podem aparecer no ledger histórico quando uma versão anterior
// não conseguiu enriquecer o mercado; nesse caso, o título fresco da API
// deve substituir o ticker.
if (/^KX[A-Z0-9]/i.test(s)) return true;
// Também trata como antigo o texto bruto de mercados esportivos,
// mesmo quando ele contém o confronto (ex.: "Trier vs Leipzig Winner?").
if (/\b(?:winner\??|to\s+advance|advances?)\s*$/i.test(s) && /\bvs\.?\b|\bversus\b|\b@\b/i.test(s)) return true;
if (/\bwill\s+(?:over|under)\b.*\bgoals?\b/i.test(s) && /\bvs\.?\b|\bversus\b|\b@\b/i.test(s)) return true;
if (/\breg(?:ulation)?\s*time\b.*\bto\s+win\b/i.test(s) && /\bvs\.?\b|\bversus\b|\b@\b/i.test(s)) return true;
return /^(will|over|under|yes|no|winner|total|moneyline)\b/i.test(s) && !/\bvs\.?\b|\bversus\b|\b@\b/i.test(s);
}
function mesclarLedgerFechadas(frescas, persistentes) {
const map = new Map();
// A memória permanente é a autoridade para identidade/metadados já confirmados.
for (const d of (persistentes || [])) map.set(chaveLedgerFechada(d), d);
for (const d of (frescas || [])) {
const k = chaveLedgerFechada(d);
const old = map.get(k);
if (!old) { map.set(k, d); continue; }
const merged = { ...d, ...old };
// Números financeiros: sempre atualizados pela API atual.
for (const key of ['realizedPnl','initialValue','totalBought','size','avgPrice','cashPnl']) {
if (d?.[key] != null) merged[key] = d[key];
}
// Identidade: depois que existe um dado bom, uma resposta oscilante não o troca.
if (String(d?._platform || old?._platform || '').toLowerCase() === 'kalshi') merged.title = d.title || old.title;
else if (!old.title || tituloFracoLedgerFrontend(old.title)) merged.title = d.title || old.title;
// Metadados de data/horário vindos da API atual são superiores aos
// metadados antigos do ledger. Isso corrige registros históricos que
// foram salvos antes do enriquecimento da Kalshi sem mexer nos valores
// financeiros da aposta.
if (d._startDate) merged._startDate = d._startDate;
if (Number(d.timestamp) > 0) merged.timestamp = d.timestamp;
if (!old._eventTicker) merged._eventTicker = d._eventTicker || old._eventTicker;
if (!old._matchupKey) merged._matchupKey = d._matchupKey || old._matchupKey;
if (!old._settledTime && d._settledTime) merged._settledTime = d._settledTime;
if (BettyAudit.timestampMs(old._resolvedTime)) merged._resolvedTime = old._resolvedTime;
else if (BettyAudit.timestampMs(d._resolvedTime)) merged._resolvedTime = d._resolvedTime;
// Versões antigas guardavam apenas a categoria raiz "Sports" da Kalshi.
// Quando a API atual traz a modalidade do milestone, ela deve prevalecer.
if (d._sportMetaResolved) {
merged.sport = d.sport || old.sport;
merged._sportMetaResolved = true;
}
if (d._sportMetaChecked) merged._sportMetaChecked = true;
map.set(k, merged);
}
return [...map.values()];
}
function entradasLedgerFechadas(lista) {
const porChave = new Map();
for (const d of (lista || [])) {
const chave = chaveLedgerFechada(d);
const separador = chave.indexOf('|');
if (separador < 0) continue;
porChave.set(chave, { platform: chave.slice(0, separador), betKey: chave.slice(separador + 1) });
}
return [...porChave.values()];
}
async function carregarAssinaturasLedgerFechadas(candidatas) {
const entries = entradasLedgerFechadas(candidatas);
if (!entries.length) return new Map();
const encontradas = new Map();
// O servidor divide as chaves em lotes de 400 para o SQLite.
const tamanhoLote = 1200;
for (let inicio = 0; inicio < entries.length; inicio += tamanhoLote) {
const r = await fetch('/api/history/closed/lookup', {
method: 'POST', headers: { 'Content-Type': 'application/json', 'Accept': 'application/json' },
body: JSON.stringify({ entries: entries.slice(inicio, inicio + tamanhoLote) }), cache: 'no-store'
});
if (!r.ok) throw new Error('consulta incremental do ledger HTTP ' + r.status);
const d = await r.json();
if (!d?.ok || !Array.isArray(d.entries)) throw new Error('consulta incremental do ledger inválida');
for (const entry of d.entries) {
const chave = String(entry?.platform || '') + '|' + String(entry?.betKey || '');
if (chave !== '|') encontradas.set(chave, entry);
}
}
return encontradas;
}
// Compatibilidade de migração: só é chamada quando o índice ainda não existe
// ou quando uma versão derivada (categoria/título) precisa ser migrada.
// Refreshes normais nunca leem o histórico inteiro.
async function carregarLedgerFechadasParaMigracao() {
const r = await fetch('/api/history/closed', { cache: 'no-store' });
if (!r.ok) throw new Error('ledger de migração HTTP ' + r.status);
const d = await r.json();
if (!d?.ok || !Array.isArray(d.fechados)) throw new Error('ledger de migração inválido');
return d.fechados;
}
function serializarLedgerEstavel(valor) {
if (valor == null || typeof valor !== 'object') return JSON.stringify(valor);
if (Array.isArray(valor)) return '[' + valor.map(serializarLedgerEstavel).join(',') + ']';
return '{' + Object.keys(valor).filter(k => valor[k] !== undefined).sort()
.map(k => JSON.stringify(k) + ':' + serializarLedgerEstavel(valor[k])).join(',') + '}';
}
function assinaturaFonteLedger(row) {
// A assinatura compara a resposta enriquecida da fonte sem trazer o JSON
// inteiro do SQLite de volta ao navegador. Ela detecta tanto liquidações
// novas quanto alterações posteriores nos registros já existentes.
// _order é somente a posição transitória na resposta/lista. Uma aposta nova
// pode deslocar todas as antigas sem alterar nenhum dado delas. Mantemos
// todos os demais campos na comparação e não modificamos a linha original.
const { _order, ...dados } = row || {};
const texto = serializarLedgerEstavel(dados);
let a = 2166136261, b = 0x9e3779b9;
for (let i = 0; i < texto.length; i++) {
const c = texto.charCodeAt(i);
a = Math.imul(a ^ c, 16777619);
b = Math.imul(b ^ c, 2246822519);
}
return texto.length + ':' + (a >>> 0).toString(36) + ':' + (b >>> 0).toString(36);
}
function linhasAlteradasNoLedger(frescas, assinaturasPersistidas) {
const delta = [];
for (const row of (frescas || [])) {
const anterior = assinaturasPersistidas?.get(chaveLedgerFechada(row));
const assinatura = assinaturaFonteLedger(row);
if (!anterior || anterior.sourceSignature !== assinatura || !anterior.metricsReady || (row._platform === 'kalshi' && anterior.titleVersion !== VERSAO_INDICE_TITULO)) {
delta.push({ ...row, _ledgerSourceSignature: assinatura });
}
}
return delta;
}
function* lotesEnvioLedgerFechadas(lista, titleVersion) {
// O limite HTTP é em bytes, não em apostas. Metadados de um único evento
// podem tornar 80 linhas maiores que os 8 MiB aceitos pelo servidor.
const limiteBytes = 1024 * 1024;
const encoder = new TextEncoder();
const prefixo = '{"fechados":[';
const sufixo = ']' + (titleVersion == null ? '' : ',"titleVersion":' + JSON.stringify(titleVersion)) + '}';
const baseBytes = encoder.encode(prefixo + sufixo).byteLength;
let linhas = [], textos = [], bytes = baseBytes;
for (const row of lista) {
const texto = JSON.stringify(row);
const tamanho = encoder.encode(texto).byteLength;
if (linhas.length && (linhas.length >= 80 || bytes + tamanho + 1 > limiteBytes)) {
yield {linhas,body:prefixo + textos.join(',') + sufixo};
linhas = []; textos = []; bytes = baseBytes;
}
bytes += tamanho + (linhas.length ? 1 : 0);
linhas.push(row); textos.push(texto);
}
if (linhas.length) yield {linhas,body:prefixo + textos.join(',') + sufixo};
}
async function enviarLoteLedgerFechadas(url, lote, titleVersion) {
const r = await fetch(url, {
method: 'POST',
headers: { 'Content-Type': 'application/json', 'Accept': 'application/json' },
body: lote.body,
cache: 'no-store'
});
if (r.status === 413 && lote.linhas.length > 1) {
// Compatibilidade com instalações/proxies que tenham um limite menor.
const meio = Math.ceil(lote.linhas.length / 2), confirmadas = [];
for (const parte of [lote.linhas.slice(0,meio),lote.linhas.slice(meio)]) {
for (const menor of lotesEnvioLedgerFechadas(parte,titleVersion)) {
const rows = await enviarLoteLedgerFechadas(url,menor,titleVersion);
for (const row of rows) confirmadas.push(row);
}
}
return confirmadas;
}
if (!r.ok) {
const erro = new Error(r.status === 413
? 'Uma aposta contém dados maiores que o limite de envio. O histórico foi preservado, mas a sincronização não foi concluída.'
: 'Não foi possível salvar o histórico: HTTP ' + r.status);
erro.httpStatus = r.status;
throw erro;
}
const d = await r.json();
if (!d?.ok || !Array.isArray(d.fechados)) throw new Error('Confirmação do histórico inválida');
if (titleVersion != null) {
const chaves = new Set(d.fechados.map(chaveLedgerFechada));
if (lote.linhas.some(row=>!chaves.has(chaveLedgerFechada(row)))) throw new Error('O servidor não confirmou todas as apostas enviadas');
}
return d.fechados;
}
async function salvarLedgerFechadasLegado(lista, fallback) {
if (!Array.isArray(lista) || !lista.length) return fallback || [];
let confirmadas = [];
for (const lote of lotesEnvioLedgerFechadas(lista)) {
confirmadas = await enviarLoteLedgerFechadas('/api/history/closed',lote);
}
return confirmadas;
}
async function salvarLedgerFechadas(lista, legado = []) {
if (!Array.isArray(lista) || !lista.length) return [];
try {
const confirmadas = [];
for (const lote of lotesEnvioLedgerFechadas(lista,VERSAO_INDICE_TITULO)) {
const rows = await enviarLoteLedgerFechadas('/api/history/closed/delta',lote,VERSAO_INDICE_TITULO);
for (const row of rows) confirmadas.push(row);
}
return confirmadas;
} catch (e) {
// Só servidores antigos usam compatibilidade. Falhas de rede, payload ou
// banco precisam interromper a confirmação, jamais virar um delta vazio.
if (e.httpStatus !== 404 && e.httpStatus !== 405) throw e;
console.warn('Ledger incremental indisponível; usando compatibilidade:', e);
await salvarLedgerFechadasLegado(legado.length ? legado : lista, []);
return [];
}
}
// Compatibilidade com chamadas antigas; não é mais usado para fechar o ciclo.
function salvarLedgerFechadasAsync(lista) { salvarLedgerFechadas(lista, lista).catch(()=>{}); }
function chaveIndiceHistorico(d) {
const chave = chaveLedgerFechada(d);
const separador = chave.indexOf("|");
return { platform: separador >= 0 ? chave.slice(0, separador) : "polymarket", betKey: separador >= 0 ? chave.slice(separador + 1) : chave };
}
function ymdHistoricoDb(data) {
if (!data || Number.isNaN(data.getTime())) return "";
ymdHistoricoDb.formato ||= new Intl.DateTimeFormat("en-CA", {
timeZone: "America/Sao_Paulo", year: "numeric", month: "2-digit", day: "2-digit"
});
return ymdHistoricoDb.formato.format(data);
}
function indiceHistoricoFechado(d) {
const chave = chaveIndiceHistorico(d);
const inicioEstrito = parseApiDate(d?._startDate || null);
const tsAquisicao = Object.hasOwn(CASAS_NOVAS, d._platform) ? BettyAudit.timestampMs(d.timestamp) : null;
const dataAquisicao = tsAquisicao ? new Date(tsAquisicao) : null;
const dataTabela = d._platform === "predict" ? dataAquisicao : dataInicioAposta(d) || dataAquisicao;
const dataPnl = getTsResultadoPnL(d);
const titulo = tituloExibicao(d);
const pnl = Number(d?.realizedPnl || 0) || 0;
const stake = Number(getStake(d)) || 0;
const odd = Number(getOddNum(d)) || 0;
return {
...chave,
category: classificarAposta(d),
startYmd: ymdHistoricoDb(inicioEstrito),
tableYmd: ymdHistoricoDb(dataTabela),
tableTs: dataTabela ? dataTabela.getTime() : 0,
pnlTs: dataPnl || 0,
pnlYmd: ymdHistoricoDb(dataPnl ? new Date(dataPnl) : null),
pnlValue: pnl,
stakeValue: stake,
oddValue: odd,
won: pnl > 0 ? 1 : 0,
sourceSignature: d?._ledgerSourceSignature || assinaturaFonteLedger(d),
metricsReady: 1,
titleSearch: titulo.toLocaleLowerCase("pt-BR"),
titleDisplay: titulo, titleVersion: versaoTituloConfirmado(d,titulo)
};
}
// A categoria é um campo derivado do índice SQL. Quando a regra de
// classificação muda, reindexamos o ledger canônico uma única vez para que
// Histórico, Dashboard, Estatísticas e CSV passem a mostrar a mesma modalidade.
const VERSAO_INDICE_CATEGORIA = "11";
// Títulos são dados derivados do ledger, assim como a categoria. Esta versão
// permite corrigir somente title_display/title_search uma única vez, sem tocar
// em datas, PnL, stakes, métricas ou classificação já persistida.
const VERSAO_INDICE_TITULO = "28";
function versaoTituloConfirmado(d,titulo) {
if (String(d?._platform||'').toLowerCase() !== 'kalshi')return '';
if (globalThis.BettyMarketTitles.kalshiTitleMissingMatchup(d,titulo))return '';
if (globalThis.BettyMarketTitles.kalshiSpecialTitle(d))return VERSAO_INDICE_TITULO;
if (globalThis.BettyMarketTitles.inningWinnerTitle(d))return VERSAO_INDICE_TITULO;
if (d._titleDisplayVersion === VERSAO_INDICE_TITULO)return VERSAO_INDICE_TITULO;
if (!d._kalshiTitleMeta)return '';
const meta=d._kalshiTitleMeta;
const parsed=globalThis.BettyMarketTitles?.kalshiDetail(meta.market,String(d.outcome||'').toLowerCase(),meta.scope);
return titulo !== d.title || parsed || /^.+?\s+wins?\??$/i.test(String(meta.market?.title||'')) ? VERSAO_INDICE_TITULO : '';
}
// Datas de cada mercado Kalshi passaram a ser individuais. Esta versão força
// uma única reconstrução do índice derivado, sem mudar o ledger canônico.
const VERSAO_INDICE_DATA = "3";
function chaveVersaoIndiceCategoria() {
return "betty:categoryIndexVersion:" + (BETTY_USER?.id || "local");
}
function chaveVersaoIndiceTitulo() {
return "betty:titleIndexVersion:" + (BETTY_USER?.id || "local");
}
function chaveVersaoIndiceData() {
return "betty:dateIndexVersion:" + (BETTY_USER?.id || "local");
}
// As versões com que o índice foi montado ficam no SQLite, junto do índice
// (`/index/status`). No localStorage, um perfil novo do WebView fazia o índice
// inteiro ser refeito. O navegador só é lido quando o banco ainda não tem a
// versão, para trazer a marca de quem já tinha o índice montado.
let versoesIndiceHistoricoDb = null;
const VERSOES_INDICE_HISTORICO = {
category: () => [chaveVersaoIndiceCategoria(), VERSAO_INDICE_CATEGORIA],
title: () => [chaveVersaoIndiceTitulo(), VERSAO_INDICE_TITULO],
date: () => [chaveVersaoIndiceData(), VERSAO_INDICE_DATA]
};
function versaoIndiceRegistrada(tipo) {
const doBanco = versoesIndiceHistoricoDb?.[tipo];
if (doBanco) return doBanco;
try { return localStorage.getItem(VERSOES_INDICE_HISTORICO[tipo]()[0]); }
catch (_) { return null; }
}
function categoriaIndicePrecisaReindexar() { return versaoIndiceRegistrada("category") !== VERSAO_INDICE_CATEGORIA; }
function tituloIndicePrecisaReindexar() { return versaoIndiceRegistrada("title") !== VERSAO_INDICE_TITULO; }
function dataIndicePrecisaReindexar() { return versaoIndiceRegistrada("date") !== VERSAO_INDICE_DATA; }
async function registrarVersoesIndiceHistorico(tipos) {
if (!tipos.length) return;
const versions = Object.fromEntries(tipos.map(tipo => [tipo, VERSOES_INDICE_HISTORICO[tipo]()[1]]));
try {
const res = await fetch('/api/history/closed/index/versions', {
method: 'POST', headers: { 'Content-Type': 'application/json', 'Accept': 'application/json' },
body: JSON.stringify({ versions }), cache: 'no-store'
});
const data = res.ok ? await res.json() : null;
if (data?.ok && data.versions && typeof data.versions === 'object') {
versoesIndiceHistoricoDb = data.versions;
return;
}
} catch (_) {}
// Sem o banco, guarda no navegador como antes; a próxima abertura leva ao SQLite.
for (const tipo of tipos) {
const [chave, versao] = VERSOES_INDICE_HISTORICO[tipo]();
try { localStorage.setItem(chave, versao); } catch (_) {}
}
}
// Quem já tinha o índice montado com as regras atuais antes de as versões irem
// para o SQLite leva a marca do navegador para o banco, sem refazer o índice.
async function trazerVersoesIndiceDoNavegador() {
if (!versoesIndiceHistoricoDb) return;
const tipos = Object.keys(VERSOES_INDICE_HISTORICO).filter(tipo => {
if (versoesIndiceHistoricoDb[tipo]) return false;
const [chave, versao] = VERSOES_INDICE_HISTORICO[tipo]();
try { return localStorage.getItem(chave) === versao; } catch (_) { return false; }
});
await registrarVersoesIndiceHistorico(tipos);
}
async function sincronizarTitulosIndiceHistorico() {
const base = await carregarLedgerFechadasParaMigracao();
const semMercado = base.filter(d => String(d._platform || 'polymarket').toLowerCase() === 'polymarket' && !globalThis.BettyMarketTitles.exactMarket(d));
if (semMercado.length) {
const { sportBySlug } = await buscarDatasInicioPorSlug(semMercado, { forcarSlugs: new Set(semMercado.map(d => d.eventSlug || d.slug).filter(Boolean)) });
for (const d of semMercado) {
const meta = sportBySlug[d.eventSlug] || sportBySlug[d.slug];
if (globalThis.BettyMarketTitles.exactMarket(d, meta)) d._sportMeta = meta;
}
}
_tituloExibicaoCache.clear();
const entradas = [...new Map(base.map(d => {
const chave = chaveIndiceHistorico(d);
const titulo = tituloExibicao(d);
return [chave.platform + "|" + chave.betKey, {
platform: chave.platform, betKey: chave.betKey,
titleSearch: titulo.toLocaleLowerCase("pt-BR"), titleDisplay: titulo, titleVersion: versaoTituloConfirmado(d,titulo)
}];
})).values()];
for (let inicio = 0; inicio < entradas.length; inicio += 200) {
const res = await fetch('/api/history/closed/index/titles', {
method: 'POST', headers: { 'Content-Type': 'application/json', 'Accept': 'application/json' },
body: JSON.stringify({ entries: entradas.slice(inicio, inicio + 200) }), cache: 'no-store'
});
if (!res.ok) throw new Error('títulos do índice HTTP ' + res.status);
}
}
async function gravarEntradasIndiceHistorico(base) {
const entradas = [...new Map(base.map(d => {
const indice = indiceHistoricoFechado(d);
return [indice.platform + '|' + indice.betKey, indice];
})).values()];
for (let inicio = 0; inicio < entradas.length; inicio += 200) {
const res = await fetch('/api/history/closed/index', {
method: 'POST', headers: { 'Content-Type': 'application/json', 'Accept': 'application/json' },
body: JSON.stringify({ entries: entradas.slice(inicio, inicio + 200) }), cache: 'no-store'
});
if (!res.ok) throw new Error('índice HTTP ' + res.status);
}
}
async function repararPendenciasIndiceHistorico() {
let cursor = 0;
while (true) {
const res = await fetch('/api/history/closed/index/pending?cursor=' + cursor, { cache: 'no-store' });
// Compatibilidade com um servidor antigo ainda aberto. Nunca pula pendências.
if (res.status === 404 || res.status === 405) {
await gravarEntradasIndiceHistorico(await carregarLedgerFechadasParaMigracao());
return;
}
if (!res.ok) throw new Error('pendências do índice HTTP ' + res.status);
const data = await res.json();
if (!data?.ok || !Array.isArray(data.fechados)) throw new Error('pendências do índice inválidas');
if (!data.fechados.length) return;
if (!Number.isSafeInteger(data.nextCursor) || data.nextCursor <= cursor) throw new Error('pendências do índice sem progresso');
await gravarEntradasIndiceHistorico(data.fechados);
cursor = data.nextCursor;
}
}
let ultimaRecuperacaoDatasPnl=0;
async function sincronizarIndiceHistoricoFechado(alteradas = []) {
try {
const statusRes = await fetch('/api/history/closed/index/status', { cache: 'no-store' });
const status = statusRes.ok ? await statusRes.json() : null;
versoesIndiceHistoricoDb = status?.versions && typeof status.versions === 'object' ? status.versions : null;
await trazerVersoesIndiceDoNavegador();
// Uma instalação que ainda não possui o índice faz UMA migração compatível.
// Depois disso, cada refresh indexa só as linhas confirmadamente alteradas.
// Uma nova versão das regras de categoria também reindexa tudo uma única vez.
const reindexarCategorias = categoriaIndicePrecisaReindexar();
const reindexarTitulos = tituloIndicePrecisaReindexar();
const reindexarDatas = dataIndicePrecisaReindexar();
if (reindexarDatas || (Date.now()-ultimaRecuperacaoDatasPnl>=10*60*1000 && fechados.some(d=>BettyAudit.resolvedLoss(d) && !BettyAudit.timestampMs(d._resolvedTime)))) {
ultimaRecuperacaoDatasPnl=Date.now();
const recovered = await fetch('/api/history/closed/recover-pnl-dates',{method:'POST',cache:'no-store'});
if(!recovered.ok)console.warn('Datas de resolução pendentes; nova tentativa na próxima sincronização.');
}
// Regras novas exigem migração completa. Um índice incompleto, por si só,
// exige somente o delta atual e as pendências do ledger, não todo o histórico.
const migracaoCompleta = reindexarCategorias || reindexarDatas;
const base = migracaoCompleta ? await carregarLedgerFechadasParaMigracao() : (alteradas || []);
await gravarEntradasIndiceHistorico(base);
if (!migracaoCompleta && !status?.complete) await repararPendenciasIndiceHistorico();
if (reindexarTitulos && !migracaoCompleta) await sincronizarTitulosIndiceHistorico();
const finalRes = await fetch('/api/history/closed/index/status', { cache: 'no-store' });
const finalStatus = finalRes.ok ? await finalRes.json() : null;
const completo = Boolean(finalStatus?.complete);
if (completo) {
await registrarVersoesIndiceHistorico([
reindexarCategorias && "category", reindexarTitulos && "title", reindexarDatas && "date"
].filter(Boolean));
}
return completo;
} catch (e) {
// O índice é derivado e nunca é autorizado a interromper a sincronização
// canônica. Até a próxima tentativa, a interface continua no caminho atual.
console.warn('Índice do histórico indisponível:', e);
return false;
}
}
function chaveSnapshot(d) {
const plat = String(d?._platform || 'polymarket').toLowerCase();
if (plat === 'predict' || plat === 'sxbet') return plat + '|' + String(d?.id || '');
if (plat === 'kalshi') {
return 'kalshi|' + String(d?.slug || d?.ticker || d?.market_ticker || d?.conditionId || '').trim();
}
return 'polymarket|' + chavePolymarket(d);
}
function mesclarLinhasPreferindoNovas(antigas, novas) {
const map = new Map();
for (const d of (antigas || [])) map.set(chaveSnapshot(d), d);
for (const d of (novas || [])) {
const k = chaveSnapshot(d);
const old = map.get(k);
// A resposta nova atualiza os valores, mas preserva metadados bons da antiga
// quando a API devolve uma linha parcialmente enriquecida.
map.set(k, old ? { ...old, ...d, title: (d.title && !/^Will (over|under)|^Will there/i.test(String(d.title))) ? d.title : (old.title || d.title) } : d);
}
return [...map.values()];
}
function reconciliarSnapshotPolymarket(prevOpen, nextOpen, nextClosed) {
const closedKeys = new Set((nextClosed || []).map(chaveSnapshot));
const prev = prevOpen || [];
const next = nextOpen || [];
const nextKeys = new Set(next.map(chaveSnapshot));
const prevMissing = prev.filter(d => !nextKeys.has(chaveSnapshot(d)) && !closedKeys.has(chaveSnapshot(d)));
// Se uma atualização perdeu posições sem que elas tenham aparecido como fechadas,
// mantemos o snapshot anterior. Isso protege contra oscilações temporárias da API.
return mesclarLinhasPreferindoNovas(next, prevMissing);
}
function reconciliarSnapshotKalshi(prevOpen, nextOpen, nextClosed) {
const closedKeys = new Set((nextClosed || []).map(chaveSnapshot));
const prev = prevOpen || [];
const next = Array.isArray(nextOpen) ? nextOpen : [];
const nextKeys = new Set(next.map(chaveSnapshot));
// A API da Kalshi pode responder temporariamente com lista vazia/partial durante
// atualização ou indexação. Não apagamos uma posição já confirmada até ela
// aparecer explicitamente como fechada/settled.
const prevMissing = prev.filter(d => !nextKeys.has(chaveSnapshot(d)) && !closedKeys.has(chaveSnapshot(d)));
return mesclarLinhasPreferindoNovas(next, prevMissing);
}
let syncProgressGeneration = 0;
let syncProgressHideTimer = null;
let syncProgressInitialGeneration = null;
function liberarNavegacaoInicial() {
document.body.classList.remove('initial-loading');
document.querySelectorAll('.sidebar, .main').forEach(el => { el.inert = false; });
}
function atualizarProgressoSync(generation, etapa, detalhe, resultado = '') {
if (generation !== refreshGeneration) return;
const box = document.getElementById('syncProgress');
if (!box) return;
clearTimeout(syncProgressHideTimer);
if (syncProgressInitialGeneration === null) syncProgressInitialGeneration = generation;
// Com dados salvos na tela, o selo do topo faz o papel da barra.
if (exibindoDadosSalvos) {
box.hidden = true;
atualizarSeloDadosSalvos(detalhe, resultado);
return;
}
// A barra pertence somente à primeira carga desta abertura, inclusive se ela falhar.
if (generation !== syncProgressInitialGeneration) {
box.hidden = true;
liberarNavegacaoInicial();
return;
}
syncProgressGeneration = generation;
box.hidden = false;
box.classList.toggle('compact', snapshotAoVivoConfirmado);
box.classList.toggle('complete', resultado === 'ok');
box.classList.toggle('warning', resultado === 'warning');
document.getElementById('syncProgressTitle').textContent = resultado === 'ok' ? 'Dados confirmados' : resultado === 'warning' ? 'Atualização incompleta' : snapshotAoVivoConfirmado ? 'Atualizando seus dados' : 'Preparando seus dados';
document.getElementById('syncProgressDetail').textContent = detalhe;
box.setAttribute('aria-label', detalhe);
box.style?.setProperty('--sync-progress', `${resultado === 'ok' ? 100 : [8,32,62,88][Math.max(0,Math.min(3,etapa))]}%`);
box.querySelectorAll('.sync-progress-step').forEach((step, i) => {
const done = resultado === 'ok' || i < etapa;
step.classList.toggle('done', done);
step.classList.toggle('active', !resultado && i === etapa);
step.querySelector('.sync-progress-icon').textContent = done ? '✓' : String(i + 1);
});
// Apenas encerra o indicador visual; não atrasa nem agenda consultas.
if (resultado) {
liberarNavegacaoInicial();
box.hidden = true;
}
if (resultado) syncProgressHideTimer = setTimeout(() => {
if (syncProgressGeneration === generation && generation === refreshGeneration) box.hidden = true;
}, resultado === 'ok' ? 1500 : 5000);
}
// Abre direto com o último estado gravado, quando existe: o índice do histórico
// e as posições e saldos do cache local. O estado salvo do histórico é
// congelado no backend ANTES de a sincronização começar a gravar.
async function abrirComDadosSalvos() {
let status = null;
try {
const res = await fetch('/api/history/closed/index/status', { cache: 'no-store' });
status = res.ok ? await res.json() : null;
} catch (_) {}
const temHistorico = Number(status?.indexed) > 0;
if (!temHistorico && !abertos.length) return false;
try {
const res = await fetch('/api/history/snapshot', { method: 'POST', cache: 'no-store' });
if (!res.ok) return false;
} catch (_) { return false; }
exibindoDadosSalvos = true;
indiceSalvoDisponivel = temHistorico;
for (const [casa, [ler, gravar]] of Object.entries(VARIAVEIS_SALDO)) {
const salvo = saldosSalvosDaAbertura[casa];
if (!BETTY_CONNECTIONS?.[casa] || ler() != null || !Number.isFinite(Number(salvo?.v))) continue;
gravar(Number(salvo.v));
saldosDosDadosSalvos.add(casa);
}
dadosSalvosEm = Math.max(Number(ultimoFetchTs) || 0, ...Object.values(saldosSalvosDaAbertura).map(s => Number(s?.t) || 0));
atualizarSeloDadosSalvos('');
document.getElementById('syncProgress').hidden = true;
liberarNavegacaoInicial();
atualizarSaldosFixos();
posAtualizarDados();
return true;
}
// Volta às leituras normais: a sincronização confirmou os dados (ou a conta
// mudou). Saldos que não foram reconfirmados voltam a ficar sem valor.
function encerrarDadosSalvos() {
if (!exibindoDadosSalvos) return;
exibindoDadosSalvos = false;
indiceSalvoDisponivel = false;
for (const casa of saldosDosDadosSalvos) VARIAVEIS_SALDO[casa]?.[1](null);
saldosDosDadosSalvos.clear();
// As leituras feitas no estado salvo não servem para o estado confirmado.
resumoDashboardDb = null; resumoDashboardDbChave = "";
resumoPeriodoDash = null;
resumoEstatDb = null; resumoEstatDbChave = "";
pnlHistoricoDb = null;
paginasHistoricoPosicoes.clear();
const selo = document.getElementById('seloDadosSalvos');
if (selo) selo.hidden = true;
fetch('/api/history/snapshot', { method: 'DELETE', cache: 'no-store' }).catch(() => {});
atualizarSaldosFixos();
}
function horaDadosSalvos() {
if (!dadosSalvosEm) return '';
const quando = new Date(dadosSalvosEm);
const hora = quando.toLocaleTimeString('pt-BR', { hour: '2-digit', minute: '2-digit', timeZone: 'America/Sao_Paulo' });
if (ymdBrasil(quando) === ymdBrasil(new Date())) return hora;
return quando.toLocaleDateString('pt-BR', { day: '2-digit', month: '2-digit', timeZone: 'America/Sao_Paulo' }) + ', ' + hora;
}
function atualizarSeloDadosSalvos(detalhe, resultado = '') {
const selo = document.getElementById('seloDadosSalvos');
if (!selo) return;
if (resultado === 'ok') { selo.hidden = true; return; }
const hora = horaDadosSalvos();
const falhou = resultado === 'warning';
selo.hidden = false;
selo.classList.toggle('aviso', falhou);
document.getElementById('seloDadosSalvosTexto').textContent = falhou
? (hora ? `Dados salvos de ${hora} · não foi possível atualizar` : 'Dados salvos · não foi possível atualizar')
: (hora ? `Atualizando · dados de ${hora}` : 'Atualizando · dados salvos');
selo.title = falhou
? 'Os números são os últimos salvos. Use Atualizar agora para tentar de novo.'
: (detalhe || 'Os números são os últimos salvos e serão trocados pelos confirmados ao fim da atualização.');
}
async function carregarAba(aba, opcoes = {}) {
if (carregandoAba && !opcoes.forcar) return;
const generation = ++refreshGeneration;
let versaoSincronizacaoIndice = opcoes.versaoIndice ?? null;
atualizarProgressoSync(generation, 0, 'Buscando posições nas casas conectadas…');
carregandoAba = true;
// Indicador textual abaixo do "Online": avisa que uma atualização está
// em andamento sem tocar nas caixas de estatísticas. Volta a mostrar
// "Atualizado há Xs" automaticamente e de forma instantânea assim que
// a atualização terminar (sucesso ou falha), via atualizarStatusOnline().
mostrandoAtualizando = true;
const statusAtualizadoEl = document.getElementById("statusAtualizado");
if (statusAtualizadoEl) statusAtualizadoEl.textContent = "Atualizando...";
for (const casa of CASAS_STAKE) {
const connected = Boolean(BETTY_CONNECTIONS?.[casa]);
registrarSyncCasa(casa, { connected, state: connected ? "syncing" : "disconnected", lastError: null });
}
atualizarStatusOnline(false);
// Durante um refresh, mantenha as últimas métricas confirmadas na tela.
// Nunca substituímos as caixas por "—" enquanto a API está atualizando.
// O novo snapshot só passa a valer quando for confirmado mais abaixo.
if (!snapshotAoVivoConfirmado && !abertos.length && !fechados.length) {
if (viewAtual === "estatisticas") renderEstatisticas();
else renderizarStats();
}
const content = document.getElementById("content");
const btn = document.getElementById("btnAtualizarTabela");
if (!abertos.length && !fechados.length && content) {
content.innerHTML = '<div class="loading">Carregando posições...</div>';
}
if (btn) btn.classList.add("spinning");
// Snapshot usado somente como fallback por plataforma. Nunca fazemos merge
// incremental do mesmo conjunto, evitando acumulação/duplicação a cada ciclo.
const podePoly = !!BETTY_CONNECTIONS?.polymarket;
const podeKalshi = !!BETTY_CONNECTIONS?.kalshi;
const prev = {
polyOpen: podePoly ? abertos.filter(d => casaDaAposta(d) === "polymarket") : [],
kalshiOpen: podeKalshi ? abertos.filter(d => (d._platform || "").toLowerCase() === "kalshi") : [],
polyClosed: podePoly ? fechados.filter(d => casaDaAposta(d) === "polymarket") : [],
kalshiClosed: podeKalshi ? fechados.filter(d => (d._platform || "").toLowerCase() === "kalshi") : []
};
try {
// Busca as duas fontes em paralelo. Fechadas aguardam ESTE MESMO snapshot
// de abertas antes de incluir perdas resolvidas e corrigir o fluxo financeiro.
for (const entry of performance.getEntriesByType('measure')) {
if (entry.name.startsWith('sync:')) performance.clearMeasures(entry.name);
}
const cronometrar = (etapa, consultar) => {
const inicio = performance.now();
return consultar().finally(() => performance.measure('sync:' + etapa, { start: inicio }));
};
const novasCasas = casasConectadas().filter(casa => Object.hasOwn(CASAS_NOVAS, casa));
if (novasCasas.length) versaoSincronizacaoIndice ??= iniciarSincronizacaoIndiceHistorico();
const novasCasasPromise = Promise.all(novasCasas.map(async casa => {
try {
const data = await cronometrar(NOMES_CASAS[casa], async () => {
const response = await fetch("/api/tracked/" + casa + "/sync", { method: "POST", cache: "no-store" });
const result = await response.json();
if (!response.ok || !result.ok || !Array.isArray(result.open) || !Array.isArray(result.recent)) {
throw new Error(result.error || "Resposta de sincronização inválida.");
}
return result;
});
return { casa, data };
} catch (error) {
return { casa, error };
}
}));
const polyOpenPromise = cronometrar('Polymarket abertas', () => fetchPosicoesAbertas())
.then(v => ({ ok: true, value: v }))
.catch(error => ({ ok: false, error }));
const polyClosedPromise = cronometrar('Polymarket fechadas e fluxo', () => fetchPosicoesFechadas(polyOpenPromise.then(r => r.ok ? r.value : null)))
.then(v => ({ ok: true, value: v }))
.catch(error => ({ ok: false, error }));
// Start independent requests now; publication still waits for the complete pipeline.
if (podePoly) buscarFluxoPolymarket().catch(()=>{});
const kalshiClosedPromise = BETTY_CONNECTIONS?.kalshi
? cronometrar('Kalshi fechadas', fetchKalshiFechados).then(v => ({ ok: true, value: v })).catch(error => ({ ok: false, error }))
: Promise.resolve({ ok: true, value: [] });
// Kalshi desconectada = não faz nenhuma chamada para /api/kalshi.
const kalshiOpenPromise = BETTY_CONNECTIONS?.kalshi
? cronometrar('Kalshi abertas', fetchKalshiAbertos).then(v => ({ ok: true, value: v })).catch(error => ({ ok: false, error }))
: Promise.resolve({ ok: true, value: [] });
const kalshiBalancePromise = BETTY_CONNECTIONS?.kalshi
? cronometrar('Kalshi saldo', fetchKalshiSaldo).then(v => ({ ok: true, value: v })).catch(error => ({ ok: false, error }))
: Promise.resolve({ ok: true, value: null });
const polyBalancePromise = BETTY_CONNECTIONS?.polymarket
? cronometrar('Polymarket saldo', fetchPolymarketSaldo).then(v => ({ ok: true, value: v })).catch(error => ({ ok: false, error }))
: Promise.resolve({ ok: true, value: null });
const predictBalancePromise = BETTY_CONNECTIONS?.predict
? cronometrar('Predict.fun saldo', fetchPredictSaldo).then(value => ({ ok: true, value })).catch(() => ({ ok: false, value: null }))
: Promise.resolve({ ok: true, value: null });
const sxBetBalancePromise = BETTY_CONNECTIONS?.sxbet
? cronometrar('SX Bet saldo', fetchSxBetSaldo).then(data => data.balance).catch(() => null)
: null;
const [polyOpenResult, kalshiOpenResult, kalshiBalanceResult, polyBalanceResult, predictBalanceResult, sxBetBalance] = await Promise.all([
polyOpenPromise, kalshiOpenPromise, kalshiBalancePromise, polyBalancePromise, predictBalancePromise, sxBetBalancePromise
]);
if (generation !== refreshGeneration) return;
// Saldo disponível real da Polymarket: pUSD da wallet conectada na Polygon.
// Não substituímos um valor confirmado por cálculo baseado no histórico.
if (polyBalanceResult.ok && Number.isFinite(polyBalanceResult.value)) {
POLY_API_BALANCE = polyBalanceResult.value;
saldosDosDadosSalvos.delete("polymarket");
} else if (!BETTY_CONNECTIONS?.polymarket) {
POLY_API_BALANCE = null;
}
// Saldo disponível real da Kalshi: vem do endpoint /portfolio/balance.
// Se a chamada falhar, preservamos o último valor confirmado.
if (kalshiBalanceResult.ok && Number.isFinite(kalshiBalanceResult.value)) {
KALSHI_API_BALANCE = kalshiBalanceResult.value;
saldosDosDadosSalvos.delete("kalshi");
} else if (!BETTY_CONNECTIONS?.kalshi) {
KALSHI_API_BALANCE = null;
}
if (predictBalanceResult.ok) { PREDICT_API_BALANCE = predictBalanceResult.value; saldosDosDadosSalvos.delete("predict"); }
saldoPredictFalhou = !predictBalanceResult.ok;
if (sxBetBalance !== null || !BETTY_CONNECTIONS?.sxbet) { SXBET_API_BALANCE = sxBetBalance; saldosDosDadosSalvos.delete("sxbet"); }
atualizarSaldosFixos();
salvarSaldosLocais();
if (generation !== refreshGeneration) return;
const polyOpen = polyOpenResult.ok ? polyOpenResult.value : prev.polyOpen;
const kalshiOpen = kalshiOpenResult.ok ? kalshiOpenResult.value : prev.kalshiOpen;
// Fechadas são atualizadas em TODO refresh. Isso é importante porque uma
// aposta pode sair de /positions e entrar em /closed-positions enquanto
// a página permanece aberta por horas.
const [polyClosedResult, kalshiClosedResult] = await Promise.all([polyClosedPromise, kalshiClosedPromise]);
if (generation !== refreshGeneration) return;
if (podePoly && polyClosedResult.ok) polyClosedSnapshotCache = { at: Date.now(), data: polyClosedResult.value || [] };
if (podeKalshi && kalshiClosedResult.ok) kalshiClosedSnapshotCache = { at: Date.now(), data: kalshiClosedResult.value || [] };
const resultadosNovasCasas = await novasCasasPromise;
if (generation !== refreshGeneration) return;
atualizarProgressoSync(generation, 1, 'Conferindo resultados e conciliando o histórico…');
// Cada plataforma é atualizada de forma atômica. Se a API oscilar e devolver
// menos posições sem que elas tenham sido fechadas, preservamos o snapshot anterior.
let nextPolyOpen = (polyOpen || []).filter(d => !isPerdaResolvida(d));
let nextKalshiOpen = Array.isArray(kalshiOpen) ? kalshiOpen : [];
let nextPolyClosed = polyClosedResult.ok ? (polyClosedResult.value || []) : prev.polyClosed;
let nextKalshiClosed = kalshiClosedResult.ok ? (kalshiClosedResult.value || []) : prev.kalshiClosed;
if (polyOpenResult.ok && polyClosedResult.ok) {
syncExportAt.polymarket = new Date().toISOString();
nextPolyOpen = reconciliarSnapshotPolymarket(prev.polyOpen, nextPolyOpen, nextPolyClosed);
// Histórico nunca deve desaparecer por uma resposta parcial da API.
nextPolyClosed = mesclarLinhasPreferindoNovas(prev.polyClosed, nextPolyClosed);
}
if (kalshiOpenResult.ok && kalshiClosedResult.ok) {
syncExportAt.kalshi = new Date().toISOString();
nextKalshiOpen = reconciliarSnapshotKalshi(prev.kalshiOpen, nextKalshiOpen, nextKalshiClosed);
// Mantém fechadas já confirmadas se uma resposta parcial vier vazia.
nextKalshiClosed = mesclarLinhasPreferindoNovas(prev.kalshiClosed, nextKalshiClosed);
}
// O cache de ~10 min da reconciliação por /activity vale SEMPRE, mesmo
// em refresh forçado (F5/"Atualizar agora"). "forcar" continua fazendo
// a busca normal de posições/fechadas acontecer na hora; só a
// verificação extra de "mercados sumidos" (rara e cara — várias
// chamadas sequenciais à API) respeita o intervalo normal. Isso não
// afeta o histórico: apostas fechadas continuam garantidas pelo ledger SQLite
// e pela consulta paginada, que não dependem desse snapshot temporário.
if (polyOpenResult.ok && polyClosedResult.ok && (!polyActivityCache.at || Date.now() - polyActivityCache.at >= POLY_ACTIVITY_CACHE_MS)) {
try {
const rec = await cronometrar('Reconciliação', () => reconciliarPolymarketPorActivity(nextPolyOpen, nextPolyClosed));
nextPolyOpen = rec.abertos;
nextPolyClosed = rec.fechados;
} catch (e) {
console.warn('Reconciliação Polymarket ignorada:', e);
}
}
nextPolyClosed = normalizarDatasPolymarketPersistidas(nextPolyClosed);
const candidatasFechadas = dedupePolymarket(nextPolyClosed).concat(nextKalshiClosed);
atualizarProgressoSync(generation, 2, 'Salvando alterações e preparando o histórico…');
// A fonte externa ainda pode ser lida integralmente para detectar mudanças,
// mas o navegador consulta do SQLite somente assinaturas compactas. Assim não
// reconstrói o histórico completo em memória a cada refresh.
const assinaturasLedger = await cronometrar('SQLite assinaturas', () => carregarAssinaturasLedgerFechadas(candidatasFechadas));
const deltaLedger = linhasAlteradasNoLedger(
candidatasFechadas,
 assinaturasLedger
);
// O servidor é a fonte persistente. Primeiro gravamos o snapshot/ledger e
// esperamos a confirmação. Só depois publicamos `fechados` na tela.
// Assim um refresh não consegue apagar uma aposta que acabou de aparecer
// porque o POST ainda estava pendente.
const inicioGravacaoLedger = performance.now();
const ledgerConfirmado = await salvarLedgerFechadas(deltaLedger, candidatasFechadas);
performance.measure('sync:SQLite ledger', { start: inicioGravacaoLedger });
// O índice recebe a versão canônica devolvida para o delta pelo servidor.
// Isso preserva títulos/datas já protegidos pelo merge, mas grava também a
// assinatura da fonte que acabou de ser observada para não reenviar a mesma
// aposta no refresh seguinte.
const assinaturaPorChave = new Map(deltaLedger.map(d => [chaveLedgerFechada(d), d._ledgerSourceSignature || assinaturaFonteLedger(d)]));
const confirmadasParaIndice = ledgerConfirmado.map(d => ({
...d,
_ledgerSourceSignature: assinaturaPorChave.get(chaveLedgerFechada(d)) || assinaturaFonteLedger(d)
}));
// A indexação pode ser longa em uma primeira carga ou após mudar uma regra
// derivada. A partir daqui, Dashboard, Estatísticas, gráficos e Histórico
// aguardam o índice inteiro antes de receber qualquer valor novo.
versaoSincronizacaoIndice ??= iniciarSincronizacaoIndiceHistorico();
resumoEstatDb = null;
resumoEstatDbChave = "";
pnlHistoricoDb = null;
resumoDashboardDb = null;
resumoDashboardDbChave = "";
resumoPeriodoDash = null;
historicoFechadoIndexado = await cronometrar('SQLite índice', () => sincronizarIndiceHistoricoFechado(confirmadasParaIndice));
if (generation !== refreshGeneration) return;
concluirSincronizacaoIndiceHistorico(versaoSincronizacaoIndice);
// Fechadas permanentes ficam exclusivamente no SQLite. Mantemos no navegador
// apenas eventuais resultados recentes úteis para a tela ao vivo; a tabela,
// Estatísticas, PnL e CSV consultam diretamente o banco.
const limiteAoVivoFechadas = Date.now() - 2 * 60 * 60 * 1000;
fechados = candidatasFechadas.filter(d => {
const data = dataInicioAposta(d);
return data && data.getTime() >= limiteAoVivoFechadas;
});
if (historicoFechadoIndexado) {
try { localStorage.removeItem('betty:lastConfirmedClosed:' + (BETTY_USER?.id || '')); } catch (_) {}
}
const abertosNovasCasas = [];
for (const { casa, data, error } of resultadosNovasCasas) {
const status = casaSync[casa];
if (data) {
abertosNovasCasas.push(...data.open);
fechados.push(...data.recent);
syncExportAt[casa] = new Date().toISOString();
registrarSyncCasa(casa, {
state: data.warning ? "partial" : "ok", lastOk: data.warning ? status.lastOk : Date.now(),
lastError: data.warning || null, openCount: data.open.length, closedCount: data.closedCount
});
} else {
abertosNovasCasas.push(...abertos.filter(d => casaDaAposta(d) === casa));
registrarSyncCasa(casa, { state: status.lastOk ? "stale" : "error", lastError: mensagemErroSync(error) });
}
}
abertos = dedupePolymarket(nextPolyOpen).concat(nextKalshiOpen, abertosNovasCasas);
const polyOk = !podePoly || (polyOpenResult.ok && polyClosedResult.ok);
const kalshiOk = !podeKalshi || (kalshiOpenResult.ok && kalshiClosedResult.ok);
if (podePoly) {
registrarSyncCasa("polymarket", {
connected: true,
state: polyOk ? "ok" : "stale",
lastOk: polyOk ? Date.now() : (casaSync.polymarket.lastOk || 0),
lastError: polyOk ? null : mensagemErroSync(polyOpenResult.error || polyClosedResult.error),
openCount: nextPolyOpen.length,
closedCount: (polyClosedResult.ok ? (polyClosedResult.value || []).length : casaSync.polymarket.closedCount) || 0
});
}
if (podeKalshi) {
registrarSyncCasa("kalshi", {
connected: true,
state: kalshiOk ? "ok" : "stale",
lastOk: kalshiOk ? Date.now() : (casaSync.kalshi.lastOk || 0),
lastError: kalshiOk ? null : mensagemErroSync(kalshiOpenResult.error || kalshiClosedResult.error),
openCount: nextKalshiOpen.length,
closedCount: (kalshiClosedResult.ok ? (kalshiClosedResult.value || []).length : casaSync.kalshi.closedCount) || 0
});
}
if (!podePoly) registrarSyncCasa("polymarket", { connected: false, state: "disconnected", lastError: null });
if (!podeKalshi) registrarSyncCasa("kalshi", { connected: false, state: "disconnected", lastError: null });
// As posições abertas permanecem no snapshot vivo. Fechadas usam o SQLite
// para tabela, estatísticas e PnL; `fechados` acima é apenas reconciliação.
const novasCasasOk = resultadosNovasCasas.every(({ data }) => data && !data.warning);
const recebeuNovaCasa = resultadosNovasCasas.some(({ data }) => data);
snapshotAoVivoConfirmado = snapshotAoVivoConfirmado || (polyOk || kalshiOk) &&
resultadosNovasCasas.every(({ data }) => data && (!data.warning || data.closedCount > 0));
carregado.aberto = polyOpenResult.ok || kalshiOpenResult.ok || recebeuNovaCasa || carregado.aberto;
carregado.fechado = polyClosedResult.ok || kalshiClosedResult.ok || recebeuNovaCasa || carregado.fechado;
salvarCacheLocal();
atualizarProgressoSync(generation, 3, 'Atualizando as telas com os dados recebidos…');
// As telas só saem dos dados salvos quando o snapshot vivo e o índice foram
// confirmados e a Polymarket e a Kalshi responderam. Se uma delas falhou, o
// selo avisa e os saldos salvos continuam na tela até a próxima atualização.
if (exibindoDadosSalvos && snapshotAoVivoConfirmado && historicoFechadoIndexado && polyOk && kalshiOk) encerrarDadosSalvos();
posAtualizarDados();
atualizarStatusOnline(polyOk && kalshiOk && novasCasasOk);
const syncCompleta = polyOk && kalshiOk && historicoFechadoIndexado && novasCasasOk;
atualizarProgressoSync(generation, 3, syncCompleta ? 'Dados recebidos e histórico conferido.' : 'Algumas fontes não foram confirmadas. Os últimos dados disponíveis foram preservados.', syncCompleta ? 'ok' : 'warning');
} catch (err) {
if (versaoSincronizacaoIndice != null) concluirSincronizacaoIndiceHistorico(versaoSincronizacaoIndice);
if (generation !== refreshGeneration) return;
console.error("Refresh Betty:", err);
atualizarProgressoSync(generation, 0, 'Não foi possível concluir a atualização. Os dados já salvos foram preservados.', 'warning');
for (const casa of casasConectadas()) {
registrarSyncCasa(casa, { ...casaSync[casa], connected: true, state: casaSync[casa].lastOk ? "stale" : "error", lastError: mensagemErroSync(err) });
}
atualizarStatusOnline(false);
// Mesmo sem um snapshot fechado em memória, o histórico já indexado continua
// disponível no SQLite. Uma falha externa não pode substituir essa tabela por
// uma tela de erro nem apagar o que o usuário acabou de consultar.
if (exibindoDadosSalvos || historicoFechadoIndexado || abertos.length || fechados.length) {
posAtualizarDados();
} else {
const stale = lerCacheLocal(true);
if (stale && stale.abertos.length) {
abertos = stale.abertos;
carregado.aberto = true;
posAtualizarDados();
} else if (content) {
content.innerHTML = `
<div class="error">
Não foi possível carregar os dados.<br>
<small>Falha de rede ou API bloqueada neste dispositivo.</small><br>
<button type="button" data-on-click="recarregarAbaAtual()" style="margin-top:12px;padding:8px 16px;border-radius:8px;border:1px solid var(--border);background:var(--card);color:var(--text);font-weight:600;cursor:pointer;">
Tentar de novo
</button>
</div>`;
}
}
} finally {
// Mesmo que este refresh tenha sido superado por outro, ele não pode deixar
// a interface presa em "sincronizando". A versão impede que uma conclusão
// antiga encerre uma sincronização mais nova.
if (versaoSincronizacaoIndice != null) concluirSincronizacaoIndiceHistorico(versaoSincronizacaoIndice);
if (generation === refreshGeneration) {
if (btn) btn.classList.remove("spinning");
carregandoAba = false;
agendarPreparoNavegacao();
}
}
}
function mesclarPerdasNasFechadas(fechadas, posicoesAbertasRaw) {
const perdas = (posicoesAbertasRaw || [])
.filter(isPerdaResolvida)
.map((d, i) => perdaComoFechada(d, i));
if (!perdas.length) return fechadas;
const keys = new Set(fechadas.map(d =>
(d.conditionId || "") + "|" + (d.outcome || d.outcomeIndex || "")
));
const novas = perdas.filter(d =>
!keys.has((d.conditionId || "") + "|" + (d.outcome || d.outcomeIndex || ""))
);
return fechadas.concat(novas);
}
function chavePolymarket(d) {
return [d.conditionId || '', d.asset || '', d.outcomeIndex ?? d.outcome ?? ''].join('|');
}
function dedupePolymarket(lista) {
const out = [];
const seen = new Set();
for (const d of (lista || [])) {
const k = chavePolymarket(d);
if (seen.has(k)) continue;
seen.add(k);
out.push(d);
}
return out;
}
async function buscarActivityPolymarket() {
const cache = polyFluxoCache;
const flow = await buscarFluxoPolymarket();
if (cache !== polyFluxoCache || !flow.complete) throw new Error('Activity obsoleta ou incompleta');
polyActivityCache = { at: cache.at, conditionIds: [...flow.conditionIds] };
try { localStorage.setItem(POLY_ACTIVITY_AT_KEY, String(polyActivityCache.at)); } catch (_) {}
return polyActivityCache.conditionIds;
}
// PnL por aposta reconstruido a partir do caixa de cada mercado.
// A API /closed-positions devolve avgPrice, totalBought e realizedPnl corrompidos
// em parte dos mercados — aparecem perdas MAIORES que o valor apostado, o que e
// impossivel numa posicao comprada. O historico de /activity tem as compras,
// vendas e resgates reais, e reconstroi o resultado exato: receita - custo.
// YIELD nao entra: e rendimento da conta, nao resultado de uma aposta.
let polyFluxoCache = { at: 0, porMercado: null };
const POLY_FLUXO_CACHE_MS = 10 * 60 * 1000;
async function buscarFluxoPolymarket() {
if (polyFluxoCache.porMercado && Date.now() - polyFluxoCache.at < POLY_FLUXO_CACHE_MS) {
return polyFluxoCache.porMercado;
}
if (polyFluxoCache.promise) return polyFluxoCache.promise;
const cache = polyFluxoCache;
const promise = (async()=>{
const porMercado = new Map();
const conditionIds = new Set();
const seenActivity = new Set();
let complete = false;
let offset = 0;
const limit = 500;
// Keep a fixed upper bound while paginating so newer trades do not shift pages.
let windowEnd = Math.floor(Date.now() / 1000);
let overlapKeys = new Set(), overlapConsumed = new Set();
while (true) {
const url = new URL(POLY_API + "/activity", window.location.origin);
url.searchParams.set("limit", limit);
url.searchParams.set("offset", offset);
url.searchParams.set("sortBy", "TIMESTAMP");
url.searchParams.set("sortDirection", "DESC");
url.searchParams.set("start", "1");
if(windowEnd!=null)url.searchParams.set("end",String(windowEnd));
const res = await fetchComRetry(url.toString(), { cache: "no-store" });
if (!res.ok) throw new Error(`Activity HTTP ${res.status}`);
const data = await res.json();
if (!Array.isArray(data)) throw new Error("Activity inválida");
if (!data.length) { complete = true; break; }
for (const e of data) {
if (e.type === 'TRADE' && e.conditionId) conditionIds.add(e.conditionId);
const cond = String(e.conditionId || "").toLowerCase();
if (!cond) continue;
const usdc = Number(e.usdcSize) || 0;
const g = porMercado.get(cond) || { custo: 0, receita: 0, shares: 0, sharesOut: 0, lastCashOutTs: 0, assets: new Set(), ambiguous: false };
const activityKey = JSON.stringify([e.transactionHash, e.type, e.asset, e.side, e.timestamp, e.size, e.usdcSize]);
// Inclusive time windows overlap at the boundary second, not in the totals.
if(windowEnd!=null && Number(e.timestamp)===windowEnd && overlapKeys.has(activityKey) && !overlapConsumed.has(activityKey)) {
overlapConsumed.add(activityKey);continue;
}
// Identical rows can be a repeated page or distinct fills. Do not guess which.
if (seenActivity.has(activityKey)) g.ambiguous = true;
seenActivity.add(activityKey);
if (e.asset && (e.type === 'TRADE' || (e.type === 'REDEEM' && String(e.asset) !== '0'))) g.assets.add(String(e.asset));
if (!['TRADE', 'REDEEM', 'YIELD'].includes(e.type)) g.ambiguous = true;
if (e.usdcSize == null || !Number.isFinite(Number(e.usdcSize)) || Number(e.usdcSize) < 0) g.ambiguous = true;
const side = String(e.side || '').toUpperCase();
const size = Number(e.size);
if (e.type === 'TRADE' && (!['BUY','SELL'].includes(side) || !Number.isFinite(size) || !(size > 0))) g.ambiguous = true;
if (e.type === 'TRADE' && side === 'BUY') {
g.custo += usdc;
g.shares += size || 0;
} else if (e.type === 'REDEEM' || (e.type === 'TRADE' && side === 'SELL')) {
g.receita += usdc;
if (Number.isFinite(size) && size > 0) g.sharesOut += size;
g.lastCashOutTs = Math.max(g.lastCashOutTs, Number(e.timestamp) || 0);
if (e.type === 'TRADE' && e.transactionHash && e.asset && Number(e.timestamp)>Number(g.lastSale?.timestamp||0)) {
g.lastSale={transactionHash:String(e.transactionHash),asset:String(e.asset),timestamp:Number(e.timestamp)};
}
}
porMercado.set(cond, g);
}
if (data.length < limit) { complete = true; break; }
offset += limit;
if(offset>5000) {
const boundary=Math.min(...data.map(e=>Number(e.timestamp)));
if(!(boundary>0) || (windowEnd!=null && boundary>=windowEnd))throw new Error('Activity incompleta: janela temporal sem progresso');
windowEnd=boundary;offset=0;
overlapKeys=new Set(seenActivity);overlapConsumed=new Set();
}
}
if (!complete) throw new Error("Activity incompleta: limite de paginação atingido");
porMercado.complete = true;
porMercado.conditionIds = [...conditionIds];
if (cache !== polyFluxoCache) throw new Error('Activity obsoleta: carteira/cache alterado');
cache.at = Date.now(); cache.porMercado = porMercado;
return porMercado;
})();
cache.promise = promise;
try { return await promise; }
finally { if(cache.promise === promise) delete cache.promise; }
}
// Só corrigimos uma linha quando o mercado esta inteiramente liquidado e tem uma
// única linha fechada. Para CLOSED v2, o fluxo também precisa cobrir a saída
// das cotas até o último evento econômico confirmado pela API de posições.
function corrigirPnlPorFluxo(fechadas, abertasRaw, porMercado) {
if (!porMercado?.complete || !porMercado.size) return fechadas;
const comAberta = new Set();
for (const d of (abertasRaw || [])) {
if (!isPerdaResolvida(d) && Number(d.size ?? 1) > 0) comAberta.add(String(d.conditionId || "").toLowerCase());
}
const linhasPorMercado = new Map();
for (const d of (fechadas || [])) {
const c = String(d.conditionId || "").toLowerCase();
if (c) linhasPorMercado.set(c, (linhasPorMercado.get(c) || 0) + 1);
}
return (fechadas || []).map(d => {
const cond = String(d.conditionId || "").toLowerCase();
const saleFlow=porMercado.get(cond);
if (saleFlow?.lastSale && !saleFlow.ambiguous && saleFlow.assets.size===1 && saleFlow.assets.has(String(d.asset))) {
d={...d,_saleEvidence:saleFlow.lastSale};
}
if (!cond || comAberta.has(cond) || linhasPorMercado.get(cond) !== 1) return d;
const g = porMercado.get(cond);
if (!g || g.ambiguous || g.assets.size !== 1 || !g.assets.has(String(d.asset)) || !(g.custo > 0)) return d;
if (d._polyDataVersion === 2 && d.status === 'CLOSED') {
const encerramento = BettyAudit.timestampMs(d.last_event_at);
const ultimaSaida = BettyAudit.timestampMs(g.lastCashOutTs);
const compradas = Number(d.totalBought);
// A API arredonda o total de cotas de CLOSED a quatro casas decimais.
const margem = 0.00005 + g.shares * Number.EPSILON * 4;
if (!encerramento || ultimaSaida < encerramento || !(compradas > 0) ||
Math.abs(g.shares - compradas) > margem || !(g.sharesOut > 0) ||
Math.abs(g.sharesOut - g.shares) > margem || (d.realizedPnl > 0 && !(g.receita > 0))) return d;
}
const corrigido = {
...d,
_originalApiFinancials: d._originalApiFinancials || { realizedPnl: d.realizedPnl ?? null, initialValue: d.initialValue ?? null, avgPrice: d.avgPrice ?? null, totalBought: d.totalBought ?? null },
realizedPnl: g.receita - g.custo,
initialValue: g.custo,
_pnlFluxo: true
};
if (g.shares > 0) {
corrigido.totalBought = g.shares;
corrigido.avgPrice = g.custo / g.shares;
}
return corrigido;
});
}
async function buscarPosicoesPorMercadosPolymarket(conditionIds, endpoint, limit) {
const out = [];
const ids = [...new Set(conditionIds || [])];
// A API aceita lista CSV de condition IDs; mantemos lotes pequenos para URLs seguras.
for (let i = 0; i < ids.length; i += 20) {
const lote = ids.slice(i, i + 20);
let offset = 0;
while (true) {
const url = new URL(POLY_API + endpoint, window.location.origin);
url.searchParams.set("market", lote.join(','));
url.searchParams.set("limit", String(limit));
url.searchParams.set("offset", String(offset));
if (endpoint === "/positions") {
url.searchParams.set("sizeThreshold", "0");
url.searchParams.set("includeArchived", "true");
} else {
url.searchParams.set("sortBy", "TIMESTAMP");
url.searchParams.set("sortDirection", "DESC");
}
const res = await fetchComRetry(url.toString(), { cache: "no-store" });
if (!res.ok) throw new Error(`Reconciliação Polymarket HTTP ${res.status}`);
const data = await res.json();
if (!Array.isArray(data)) throw new Error('Reconciliação Polymarket inválida');
out.push(...data);
if (data.length < limit) break;
offset += limit;
if (offset >= 100000) throw new Error('Reconciliação Polymarket incompleta');
}
}
return out;
}
async function reconciliarPolymarketPorActivity(abertos, fechados) {
const conditionIds = await buscarActivityPolymarket();
if (!conditionIds.length) return { abertos, fechados };
const presentes = new Set();
for (const d of (abertos || [])) if (d.conditionId) presentes.add(d.conditionId);
for (const d of (fechados || [])) if (d.conditionId) presentes.add(d.conditionId);
const faltantes = conditionIds.filter(id => !presentes.has(id));
if (!faltantes.length) return { abertos, fechados };
const [recOpen, recClosed] = await Promise.all([
buscarPosicoesPorMercadosPolymarket(faltantes, "/positions", 500),
buscarPosicoesPorMercadosPolymarket(faltantes, "/closed-positions", 50)
]);
const openBy = recOpen.map((d, i) => ({
...d,
_order: 1000000 + i,
_startDate: d._startDate || null,
_platform: "polymarket"
})).filter(d => !isPerdaResolvida(d));
const closedBy = recClosed.map((d, i) => ({
...d,
_order: 1000000 + i,
_platform: "polymarket"
}));
// Só adicionamos dados que a própria API de posições/fechadas confirmou.
// Activity serve apenas para descobrir o que conferir.
return {
abertos: dedupePolymarket((abertos || []).concat(openBy)),
fechados: dedupePolymarket((fechados || []).concat(closedBy))
};
}
async function fetchPosicoesAbertas() {
if (!BETTY_CONNECTIONS?.polymarket) return [];
// Paginação completa para a wallet configurada; não limita artificialmente em 5.000 registros.
// Limite de 500 por página (o mesmo já usado em buscarPosicoesPorMercadosPolymarket
// para este mesmo endpoint /positions) — reduz o número de idas e vindas
// sequenciais para contas com muitas posições abertas.
let todas = [];
let offset = 0;
const limit = 500;
while (true) {
const url = new URL(POLY_API + "/positions", window.location.origin);
url.searchParams.set("limit", limit);
url.searchParams.set("offset", offset);
url.searchParams.set("sizeThreshold", "0");
const res = await fetchComRetry(url.toString(), { cache: "no-store" });
if (!res.ok) throw new Error(`Erro HTTP ${res.status}`);
const data = await res.json();
if (!Array.isArray(data) || data.length === 0) break;
todas = todas.concat(data);
if (data.length < limit) break;
offset += limit;
if (offset >= 100000) throw new Error("Histórico Polymarket incompleto: limite de paginação");
}
// Data de início: Gamma events?slug= (condition_ids da API está quebrado)
const { startBySlug, metasBySlug, sportBySlug } = await buscarDatasInicioPorSlug(todas);
// NÃO varremos /trades a cada atualização. Isso era uma das principais causas da lentidão:
// a cada refresh o navegador baixava páginas inteiras do histórico apenas para descobrir
// a última negociação, e a posição/evento já contém dados suficientes para a tela.
return todas.map((d, i) => {
const dataResolvida = resolverDataInicio(d, startBySlug, metasBySlug);
return {
...d,
_order: i,
_startDate: dataResolvida.data,
_dateSource: dataResolvida.fonte,
_dateSuspect: dataResolvida.suspeita,
_resolvedTime: d._resolvedTime || metasBySlug[d.eventSlug]?.markets?.[d.slug]?.closedTime || metasBySlug[d.slug]?.markets?.[d.slug]?.closedTime || null,
// Usa o mapa enriquecido nesta mesma atualização. Reler o localStorage aqui
// fazia posições novas perderem o metadado quando a quota rejeitava o cache.
_sportMeta: sportBySlug[d.eventSlug] || sportBySlug[d.slug] || null,
_platform: "polymarket"
};
});
}
// endDate meia-noite UTC vira dia anterior no BR — evita usar isso como data do jogo
function dataDoSlug(slug) {
const m = (slug || "").match(/(20\d{2}-\d{2}-\d{2})/);
return m ? m[1] + "T15:00:00Z" : null;
}
// Datas confirmadas fora da Gamma para mercados cujas regras dependem de um
// evento esportivo pontual, mas cujo payload não expõe gameStartTime/eventDate.
// A chave por slug de mercado evita aplicar uma data de uma seleção às demais.
const POLYMARKET_DATE_OVERRIDES = Object.freeze({
"will-neymar-play-in-the-world-cup": "2026-06-24T22:00:00Z",
"world-cup-nation-to-reach-quarterfinals|will-france-reach-the-quarterfinals-at-the-2026-fifa-world-cup-20260602145134499": "2026-07-04T21:00:00Z",
});
function dataMs(raw) {
const dt = parseApiDate(raw);
return dt ? dt.getTime() : 0;
}
function datasProximas(a, b, limiteMs = 24 * 60 * 60 * 1000) {
const ta = dataMs(a), tb = dataMs(b);
return ta > 0 && tb > 0 && Math.abs(ta - tb) <= limiteMs;
}
function metaDatasGamma(event) {
const campos = (obj) => ({
startTime: obj?.startTime || null, startDate: obj?.startDate || null,
gameStartTime: obj?.gameStartTime || null, eventDate: obj?.eventDate || null,
endDate: obj?.endDate || null, closedTime: obj?.closedTime || null,
umaEndDate: obj?.umaEndDate || null, creationDate: obj?.creationDate || null,
acceptingOrdersTimestamp: obj?.acceptingOrdersTimestamp || null,
description: obj?.description || "", title: obj?.title || obj?.question || "",
});
const markets = {};
for (const market of (event?.markets || [])) {
if (market?.slug) markets[market.slug] = campos(market);
}
return { event: campos(event), markets };
}
function metaDoMercado(d, meta) {
return meta?.markets?.[d?.slug] || null;
}
function inicioAdministrativo(meta, dataPadrao) {
if (!meta) return false;
const event = meta.event || {};
const market = meta.market || {};
const aberturas = [
market.creationDate, event.creationDate,
market.acceptingOrdersTimestamp, event.acceptingOrdersTimestamp,
].filter(Boolean);
const inicios = [market.startDate, event.startTime, event.startDate].filter(Boolean);
return inicios.some(inicio => datasProximas(inicio, dataPadrao)) &&
aberturas.some(abertura => inicios.some(inicio => datasProximas(inicio, abertura)));
}
function eMercadoDeCampeao(d, meta, inicio, fim) {
const texto = [d?.title, meta?.event?.title, meta?.event?.description, meta?.market?.description]
.filter(Boolean).join(" ").toLowerCase();
if (/\b(?:reach|qualif(?:y|ies)|advance|play in)\b/.test(texto)) return false;
if (!/\b(?:winner|champion|championship)\b/.test(texto)) return false;
const ti = dataMs(inicio), tf = dataMs(fim);
return ti > 0 && tf > ti && tf - ti >= 7 * 24 * 60 * 60 * 1000;
}
function resolverDataInicio(d, startBySlug, metasBySlug) {
const padrao = (d?.eventSlug && startBySlug[d.eventSlug]) || dataDoSlug(d?.eventSlug) || dataDoSlug(d?.slug) || d?._startDate || null;
const override = POLYMARKET_DATE_OVERRIDES[`${d?.eventSlug || ""}|${d?.slug || ""}`] ||
POLYMARKET_DATE_OVERRIDES[d?.eventSlug || ""] || null;
if (override) return { data: override, fonte: "override-esportivo-confirmado", suspeita: true };
// Mantém a exceção já confirmada: estes mercados representam o campeão do
// torneio e a Gamma informa endDate como o encerramento esportivo da final.
if (
d?.eventSlug === "world-cup-winner" &&
/^will\s+.+\s+win\s+the\s+2026\s+fifa\s+world\s+cup\?$/i.test(String(d?.title || "")) &&
d?.endDate
) return { data: d.endDate, fonte: "endDate-final-do-torneio", suspeita: true };
const eventMeta = metasBySlug?.[d?.eventSlug] || null;
const meta = eventMeta ? { event: eventMeta.event, market: metaDoMercado(d, eventMeta) } : null;
const suspeita = inicioAdministrativo(meta, padrao);
if (!suspeita) return { data: padrao, fonte: "startDate-evento", suspeita: false };
const dataEvento = [meta?.market?.gameStartTime, meta?.event?.gameStartTime, meta?.market?.eventDate, meta?.event?.eventDate]
.find(valor => dataMs(valor) > 0);
if (dataEvento) return { data: dataEvento, fonte: "campo-explicito-do-evento", suspeita: true };
const fim = meta?.market?.endDate || meta?.event?.endDate || d?.endDate || null;
if (fim && eMercadoDeCampeao(d, meta, padrao, fim)) {
return { data: fim, fonte: "endDate-final-do-torneio", suspeita: true };
}
// Não promovemos closedTime/umaEndDate: ambos são administrativos. Sem data
// esportiva confiável, preservamos a data atual e marcamos a suspeita.
return { data: padrao, fonte: "startDate-suspeito-sem-alternativa", suspeita: true };
}
// O ledger e o snapshot local podem ter sido gravados antes de uma regra de
// data ser enriquecida. Repassamos os registros Polymarket pelo mesmo
// resolvedor usado pela API, sem inventar uma data quando a Gamma não oferece
// uma alternativa confiável. Isso também impede que um cache antigo reverta
// uma data já corrigida no ledger.
function normalizarDatasPolymarketPersistidas(lista) {
const startBySlug = { ...lerCacheDatas() };
const metasBySlug = lerCacheMetasDatasGamma();
return (lista || []).map(d => {
if ((d?._platform || "polymarket").toLowerCase() !== "polymarket") return d;
// Para dados já persistidos, a própria data da linha é a referência inicial.
// Isso evita que o fallback de data embutida no slug troque horários corretos.
const datasDaLinha = d?.eventSlug && d?._startDate
? { ...startBySlug, [d.eventSlug]: d._startDate }
: startBySlug;
const resolvida = dataPersistidaPolymarketConfirmada(d)
? { data: d._startDate, fonte: d._dateSource, suspeita: Boolean(d._dateSuspect) }
: resolverDataInicio(d, datasDaLinha, metasBySlug);
const resolution = d._resolvedTime || metasBySlug[d.eventSlug]?.markets?.[d.slug]?.closedTime || metasBySlug[d.slug]?.markets?.[d.slug]?.closedTime || null;
if (!resolvida?.data) return d;
if (d._startDate === resolvida.data && d._dateSource === resolvida.fonte && d._dateSuspect === resolvida.suspeita && (d._resolvedTime || null) === resolution) return d;
return {
...d,
_startDate: resolvida.data,
_dateSource: resolvida.fonte,
_dateSuspect: resolvida.suspeita,
_resolvedTime: resolution
};
});
}
async function buscarDatasInicioPorSlug(lista, opcoes = {}) {
const slugs = [...new Set(lista.map(d => d.eventSlug || d.slug).filter(Boolean))];
const startBySlug = lerCacheDatas();
const metasBySlug = lerCacheMetasDatasGamma();
const sportBySlug = lerCacheEsportes();
const forcarSlugs = new Set(opcoes.forcarSlugs || []);
const linhasPorSlug = new Map();
for (const d of lista) {
const slug = d.eventSlug || d.slug;
if (!linhasPorSlug.has(slug)) linhasPorSlug.set(slug, []);
linhasPorSlug.get(slug).push(d);
}
const incompleto = s => {
if (forcarSlugs.has(s)) return true;
const meta = sportBySlug[s];
const markets = meta?.markets;
const temTokenIds = Array.isArray(markets) && markets.some(m => Array.isArray(m?.tokenIds) && m.tokenIds.length);
return !startBySlug[s] || !metasBySlug[s] || !meta || !Array.isArray(markets) || !markets.length || !temTokenIds ||
(linhasPorSlug.get(s)||[]).some(d=>!globalThis.BettyMarketTitles.exactMarket(d,meta));
};
// Recupera somente os eventos necessários; não carrega o cache inteiro do banco.
// Consultas forçadas e contratos novos continuam exigindo dados da fonte.
const candidatosCache=slugs.filter(s=>!forcarSlugs.has(s)&&incompleto(s));
const candidatosCacheSet=new Set(candidatosCache);
for(let i=0;i<candidatosCache.length;i+=400) {
try {
const response=await fetch('/api/polymarket-metadata/cache',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({slugs:candidatosCache.slice(i,i+400)}),cache:'no-store'});
if(!response.ok)continue;
const saved=await response.json();
if(saved?.ok&&Array.isArray(saved.entries))for(const entry of saved.entries) {
if(!candidatosCacheSet.has(entry.slug)||!entry.data)continue;
startBySlug[entry.slug]=entry.data.start;metasBySlug[entry.slug]=entry.data.dates;sportBySlug[entry.slug]=entry.data.sport;
}
}catch(_){ /* Cache indisponível: segue com Gamma. */ }
}
const faltando = slugs.filter(incompleto);
if (!faltando.length) return { startBySlug, metasBySlug, sportBySlug };
const novosMetadados=[];
const carregarSlug = async (slug) => {
const candidates = [slug];
if (slug.endsWith("-more-markets")) {
candidates.push(slug.slice(0, -"-more-markets".length));
}
for (const s of candidates) {
try {
const res = await fetchComRetry(
GAMMA_API + "/events?slug=" + encodeURIComponent(s)
).catch(() => null);
if (!res) continue;
if (!res.ok) continue;
const data = await res.json();
if (!Array.isArray(data) || !data.length) continue;
const event = data[0];
const start = event.startTime || event.gameStartTime || event.startDate || event.eventDate;
if (start) startBySlug[slug] = start;
metasBySlug[slug] = metaDatasGamma(event);
const meta = extrairMetaEsporteEvento(event);
if (meta && meta.texts.length) sportBySlug[slug] = meta;
if(start && meta)novosMetadados.push({slug,data:{start,dates:compactarMetasDatasGammaParaCache({[slug]:metasBySlug[slug]})[slug],sport:compactarMetaEsporteParaCache(meta)}});
if (start || meta) return;
} catch (e) { /* ignore */ }
}
};
// A correção de legados pode consultar muitos eventos uma única vez. Mantemos
// lotes moderados para não saturar a Gamma nem deixar o refresh comum com
// centenas de requisições simultâneas.
for (let inicio = 0; inicio < faltando.length; inicio += 12) {
await Promise.all(faltando.slice(inicio, inicio + 12).map(carregarSlug));
}
salvarCacheDatas(startBySlug);
salvarCacheMetasDatasGamma(metasBySlug);
salvarCacheEsportes(sportBySlug);
// Persistência limitada aos eventos consultados, não ao mapa histórico inteiro.
for(let i=0;i<novosMetadados.length;i+=12) {
try {await fetch('/api/polymarket-metadata/cache',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({entries:novosMetadados.slice(i,i+12)}),cache:'no-store'});}catch(_){ /* Melhor esforço; dados atuais já estão em memória. */ }
}
return { startBySlug, metasBySlug, sportBySlug };
}
function chaveFonteFechadaPolymarket(d) {
return [d?.conditionId || d?.condition_id || "", d?.asset || d?.tokenId || "", d?.outcomeIndex ?? d?.outcome ?? ""].join("|");
}
function assinaturaFonteFechadaPolymarket(d) {
return [
d?.eventSlug || "", d?.slug || "", d?.title || "", d?.outcome || "", d?.outcomeIndex ?? "",
d?.avgPrice ?? "", d?.initialValue ?? "", d?.totalBought ?? "", d?.size ?? "",
d?.realizedPnl ?? "", d?.cashPnl ?? "", d?.timestamp ?? "", d?.endDate ?? ""
].join("|");
}
// Fica só na memória, durante a sessão. No localStorage as linhas inteiras,
// com os metadados do evento, passavam da cota do navegador: a gravação falhava
// sem aviso depois de serializar dezenas de MB. Na abertura seguinte quem
// devolve esses metadados é o SQLite (`/api/history/closed/metadata`).
function cacheFechadasPolymarket() {
const userId = BETTY_USER?.id || "local";
if (polyClosedEnrichmentCache?.userId === userId) return polyClosedEnrichmentCache;
// Apaga o que versões antigas chegaram a gravar.
try { localStorage.removeItem(POLY_CLOSED_ENRICH_CACHE_PREFIX + userId); } catch (_) {}
polyClosedEnrichmentCache = { userId, entries: {} };
return polyClosedEnrichmentCache;
}
function fechadaPolymarketIncompleta(row) {
// Esta função valida METADADOS, não o cálculo financeiro posterior. A API de
// closed-positions pode omitir initialValue; consultar Gamma não preenche esse
// campo, que é tratado pelo fluxo financeiro sem alterar a fonte atual.
return !row || !row.title || !(row.conditionId || row.condition_id) || !(row.eventSlug || row.slug) || !row._startDate || !globalThis.BettyMarketTitles.exactMarket(row);
}
function dataFechadaPolymarketPareceTimestampAdministrativo(row) {
if (!row || String(row?._platform || "polymarket").toLowerCase() !== "polymarket") return false;
const inicio = dataMs(row._startDate);
const timestamp = Number(row.timestamp || 0) * 1000;
// Uma data esportiva não deve coincidir ao segundo com o timestamp que a API
// registra ao fechar/resgatar a posição. A margem absorve pequenas diferenças
// de serialização sem confundir o horário real de um jogo.
return inicio > 0 && timestamp > 0 && Math.abs(inicio - timestamp) <= 5 * 60 * 1000;
}
function precisaRevalidarDataFechadaPolymarket(cached) {
return Boolean(
cached?.row &&
cached.dateResolverVersion !== POLY_CLOSED_DATE_RESOLVER_VERSION &&
dataFechadaPolymarketPareceTimestampAdministrativo(cached.row)
);
}
function dataPersistidaPolymarketConfirmada(row) {
if (!(dataMs(row?._startDate) > 0) || dataFechadaPolymarketPareceTimestampAdministrativo(row)) return false;
// A suspeita se refere ao startDate ORIGINAL. Estas fontes já contêm a
// alternativa validada; manter a marca não invalida o contrato nem a data.
return !row._dateSuspect || ['campo-explicito-do-evento', 'override-esportivo-confirmado', 'endDate-final-do-torneio'].includes(row._dateSource);
}
async function enriquecerFechadasPolymarketIncremental(todas) {
const cache = cacheFechadasPolymarket();
// Cache do navegador pode expirar ou exceder a quota. O ledger é permanente.
// Só reutilizamos metadados completos do mesmo contrato/evento, nunca finanças.
const reutilizar = (d, meta) => {
if (!meta || (meta.eventSlug || meta.slug) !== (d.eventSlug || d.slug)) return false;
const row = { ...d, _platform: 'polymarket', _sportMeta: meta._sportMeta,
_startDate: meta._startDate, _dateSource: meta._dateSource,
_dateSuspect: meta._dateSuspect, _resolvedTime: d._resolvedTime || meta._resolvedTime || null };
if (fechadaPolymarketIncompleta(row) || !dataPersistidaPolymarketConfirmada(row)) return false;
cache.entries[chaveFonteFechadaPolymarket(d)] = {signature: assinaturaFonteFechadaPolymarket(d), row, at: Date.now(), dateResolverVersion: POLY_CLOSED_DATE_RESOLVER_VERSION};
return true;
};
const semMetadados = [];
for (const d of todas || []) {
const cached = cache.entries[chaveFonteFechadaPolymarket(d)];
if (cached && cached.signature === assinaturaFonteFechadaPolymarket(d) && !fechadaPolymarketIncompleta(cached.row) && !precisaRevalidarDataFechadaPolymarket(cached)) continue;
if (!precisaRevalidarDataFechadaPolymarket(cached) && reutilizar(d, cached?.row)) continue;
semMetadados.push(d);
}
for (let i=0; i<semMetadados.length; i+=400) {
const batch=semMetadados.slice(i,i+400);
try {
const response=await fetch('/api/history/closed/metadata', {method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({keys:batch.map(chaveFonteFechadaPolymarket)}),cache:'no-store'});
if (!response.ok) continue;
const result=await response.json();
if (!result?.ok || !Array.isArray(result.entries)) continue;
const metas=new Map(result.entries.map(m=>[m.betKey,m]));
for (const d of batch) {
const cached=cache.entries[chaveFonteFechadaPolymarket(d)];
if (!precisaRevalidarDataFechadaPolymarket(cached)) reutilizar(d,metas.get(chaveFonteFechadaPolymarket(d)));
}
} catch (_) { /* Sem histórico disponível, mantém a consulta normal ao Gamma. */ }
}
const pendentes = (todas || []).filter(d => {
const cached = cache.entries[chaveFonteFechadaPolymarket(d)];
return !cached || cached.signature !== assinaturaFonteFechadaPolymarket(d) ||
(fechadaPolymarketIncompleta(cached.row) && Date.now() - Number(cached.at || 0) >= POLY_INCOMPLETE_RETRY_MS) ||
precisaRevalidarDataFechadaPolymarket(cached);
});
if (pendentes.length) {
const forceSlugs = new Set();
for (const d of pendentes) {
const cached = cache.entries[chaveFonteFechadaPolymarket(d)];
if (precisaRevalidarDataFechadaPolymarket(cached)) {
const slug = d.eventSlug || d.slug;
if (slug) forceSlugs.add(slug);
}
}
for (const d of pendentes) {
const cached = cache.entries[chaveFonteFechadaPolymarket(d)];
if (cached?.row && !globalThis.BettyMarketTitles.exactMarket(cached.row)) forceSlugs.add(d.eventSlug || d.slug);
}
const { startBySlug, metasBySlug, sportBySlug } = await buscarDatasInicioPorSlug(pendentes, { forcarSlugs: forceSlugs });
for (const d of pendentes) {
const cacheKey = chaveFonteFechadaPolymarket(d);
const cached = cache.entries[cacheKey];
const revalidacaoLegada = precisaRevalidarDataFechadaPolymarket(cached);
const resolvida = resolverDataInicio(d, startBySlug, metasBySlug);
const apenasMetadado = cached?.signature === assinaturaFonteFechadaPolymarket(d) && !revalidacaoLegada && cached?.row?._startDate && !globalThis.BettyMarketTitles.exactMarket(cached.row);
const row = {
...d,
_platform: "polymarket",
_sportMeta: compactarMetaEsporteParaCache(sportBySlug[d.eventSlug] || sportBySlug[d.slug]) || null,
_resolvedTime: d._resolvedTime || metasBySlug[d.eventSlug]?.markets?.[d.slug]?.closedTime || metasBySlug[d.slug]?.markets?.[d.slug]?.closedTime || null,
_startDate: apenasMetadado ? cached.row._startDate : resolvida.data,
_dateSource: apenasMetadado ? cached.row._dateSource : resolvida.fonte,
_dateSuspect: apenasMetadado ? cached.row._dateSuspect : resolvida.suspeita
};
cache.entries[cacheKey] = {
signature: assinaturaFonteFechadaPolymarket(d),
row,
at: Date.now(),
// Apenas as linhas realmente revalidadas (ou novas) recebem a marca. Linhas
// antigas já corretas não são tocadas nem passam a gerar escrita no ledger.
dateResolverVersion: revalidacaoLegada || !cached ? POLY_CLOSED_DATE_RESOLVER_VERSION : cached.dateResolverVersion
};
}
}
return (todas || []).map((d, i) => {
const cached = cache.entries[chaveFonteFechadaPolymarket(d)]?.row;
return cached ? { ...cached, _order: i } : null;
}).filter(Boolean);
}
async function fetchPosicoesFechadas(posicoesAbertasRaw) {
if (!BETTY_CONNECTIONS?.polymarket) return [];
// A API permite até 100.000 de offset. Pagina até o fim para não perder histórico antigo.
let todas = [];
let offset = 0;
// O backend já lê o histórico inteiro da Polymarket e só fatia a lista em
// memória (aceita até 1.000 por página): 500 evita dezenas de idas e voltas.
const limit = 500;
while (true) {
const url = new URL(POLY_API + "/closed-positions", window.location.origin);
url.searchParams.set("limit", limit);
url.searchParams.set("offset", offset);
url.searchParams.set("sortBy", "TIMESTAMP");
url.searchParams.set("sortDirection", "DESC");
const res = await fetchComRetry(url.toString(), { cache: "no-store" });
if (!res.ok) throw new Error(`Erro HTTP ${res.status}`);
const data = await res.json();
if (!Array.isArray(data)) throw new Error('Histórico Polymarket inválido');
if (data.length === 0) break;
todas = todas.concat(data);
if (data.length < limit) break;
offset += limit;
if (offset >= 100000) throw new Error("Histórico Polymarket incompleto: limite de paginação");
}
// Metadados e resolução de data só são refeitos para linhas novas, alteradas
// ou incompletas. As linhas já confirmadas continuam passando pelo resolvedor
// central mais abaixo quando entram no ledger/snapshot.
let lista = await enriquecerFechadasPolymarketIncremental(todas);
// Perdas resolvidas: a API NÃO manda em closed-positions.
// Ficam em /positions com redeemable=true e curPrice≈0.
let raw = await posicoesAbertasRaw;
if (!raw) {
try {
raw = await fetchPosicoesAbertas();
} catch (e) {
raw = [];
}
}
lista = mesclarPerdasNasFechadas(lista, raw);
// Corrige PnL/stake das linhas cujo mercado a API reporta errado.
// Se /activity falhar, mantemos os valores da API em vez de perder o historico.
try {
const fluxo = await buscarFluxoPolymarket();
lista = corrigirPnlPorFluxo(lista, raw, fluxo);
} catch (e) {
console.warn("PnL por fluxo indisponivel; mantendo valores da API:", e);
}
return lista;
}
async function fetchKalshiAbertos() {
if (!BETTY_CONNECTIONS?.kalshi) return [];
const res = await fetchComRetry(KALSHI_PROXY + "/positions");
if (!res.ok) throw new Error("Kalshi positions HTTP " + res.status);
const data = await res.json();
if (!data.ok) throw new Error(data.error || "Kalshi error");
return (data.abertos || []).map(d => ({
...d,
_platform: "kalshi",
// Garante campos usados pelo site
initialValue: d.initialValue || d.market_exposure || 0,
avgPrice: d.avgPrice || (d.totalBought ? (d.initialValue || 0) / d.totalBought : null),
}));
}
async function fetchKalshiFechados() {
if (!BETTY_CONNECTIONS?.kalshi) return [];
const res = await fetchComRetry(KALSHI_PROXY + "/settlements");
if (!res.ok) throw new Error("Kalshi settlements HTTP " + res.status);
const data = await res.json();
if (!data.ok) throw new Error(data.error || "Kalshi error");
return (data.fechados || []).map(d => ({
...d,
_platform: "kalshi",
}));
}
function normalizarNomeMercadoPolymarket(d) {
  if (!d || String(d._platform || "polymarket").toLowerCase() !== "polymarket") return d?.title || "";
  const drawTitle = globalThis.BettyMarketTitles?.drawTitle(d);
  if (drawTitle) return drawTitle;
  const raw = String(d.title || "").replace(/\s+/g, " ").trim();
  if (!raw) return "";
  const combatTitle = globalThis.BettyMarketTitles?.combatTitle(d);
  if (combatTitle) return combatTitle;

  const outcomeRaw = String(d.outcome || "").replace(/\s+/g, " ").trim();
  const side = outcomeRaw.replace(/^\s*(yes|no)\s*[:\-]?\s*/i, "").trim();
  const metaTexts = Array.isArray(d?._sportMeta?.texts) ? d._sportMeta.texts : [];
  const marketMeta = Array.isArray(d?._sportMeta?.markets) ? d._sportMeta.markets : [];
  const conditionId = String(d?.conditionId || d?.condition_id || "").trim().toLowerCase();
  const marketSlug = String(d?.slug || "").trim().toLowerCase();
  const assetId = String(d?.asset || d?.assetId || d?.tokenId || "").trim();

    let selectedMarket = null;
  if (assetId) {
    const byAsset = marketMeta.filter(m =>
      (Array.isArray(m?.tokenIds) ? m.tokenIds : []).some(t => String(t).trim().replace(/^\"|\"$/g, '') === assetId.replace(/^\"|\"$/g, ''))
    );
    if (byAsset.length === 1) selectedMarket = byAsset[0];
  }
  if (!selectedMarket && conditionId) {
    const byCondition = marketMeta.filter(m => String(m?.conditionId || m?.condition_id || "").trim().toLowerCase() === conditionId);
    if (byCondition.length === 1) selectedMarket = byCondition[0];
  }
  if (!selectedMarket && marketSlug) {
    const bySlug = marketMeta.filter(m => String(m?.slug || "").trim().toLowerCase() === marketSlug);
    if (bySlug.length === 1) selectedMarket = bySlug[0];
  }

    const selectedTexts = selectedMarket ? [
    selectedMarket.question,
    selectedMarket.groupItemTitle,
    selectedMarket.sportsMarketType,
    selectedMarket.description,
    selectedMarket.category
  ].filter(Boolean).map(v => String(v).replace(/\s+/g, " ").trim()) : [];
  const marketQuestion = selectedMarket?.question ? String(selectedMarket.question).replace(/\s+/g, " ").trim() : "";
  const marketDescription = selectedMarket?.description ? String(selectedMarket.description).replace(/\s+/g, " ").trim() : "";
  const marketType = String(selectedMarket?.sportsMarketType || "").toLowerCase().replace(/[\s-]+/g, "_");
  const groupItemTitle = String(selectedMarket?.groupItemTitle || "").replace(/\s+/g, " ").trim();

  function cleanTeam(v) {
    return String(v || "")
      .replace(/\s+/g, " ")
      .replace(/^[:\-–—]+\s*/, "")
      .replace(/\s*[:\-–—]+\s*$/, "")
      .replace(/^More\s+Markets\s*[:\-–—]\s*/i, "")
      .trim();
  }
  function stripMarketSuffix(v) {
    let x = cleanTeam(v);
    x = x.replace(/\s*[-–—]\s*(?:More\s+Markets\s*[-–—]\s*)?(?:O\/U|Over\/Under|Spread|Total|Winner|Moneyline|ML)\b.*$/i, "");
    x = x.replace(/\s+(?:1st|2nd|First|Second)\s+Half\b.*$/i, "");
    x = x.replace(/\s+(?:Over|Under)\s+[-+]?\d+(?:[.,]\d+)?\b.*$/i, "");
    return cleanTeam(x);
  }
  function matchupFrom(v) {
    const source = String(v || "").replace(/\s*[–—-]\s*(?:Will|Does|Can)\b.*$/i, "").trim();
    const m = source.match(/(.+?)\s+(?:vs\.?|versus|@)\s+(.+?)$/i);
    if (!m) return "";
    let a = stripMarketSuffix(m[1]);
    let b = stripMarketSuffix(m[2]);
    const colon = b.indexOf(":");
    if (colon >= 0) b = b.slice(0, colon).trim();
    b = b.replace(/\s*[-–—]\s+.+?\b(?:O\/U|Over\/Under|Spread|Total|Winner|To Advance|Moneyline|ML)\b.*$/i, "");
    b = b.replace(/\s*(?:[–—-])\s*(?:O\/U|Over\/Under|Spread|Total|Winner|To Advance|Moneyline|ML|More\s+Markets|(?:1st|2nd|First|Second)\s+Half|(?:Over|Under)\s+\d+(?:[.,]\d+)?)\b.*$/i, "");
    b = b.replace(/\s*(?:[–—-])\s*(?:Will|Does|Can)\b.*$/i, "");
    b = b.replace(/\s+(?:Winner\??|To Advance|To Win|Advances?)(?:\s*\?)?$/i, "");
    b = b.replace(/\s+More\s+Markets\s*$/i, "");
    b = b.replace(/\s*[–—-]\s*(?:Yes|No)\s*$/i, "");
    b = b.replace(/\s*[-–—:]\s*Halftime Result\??$/i, "");
    return a && b ? `${a} vs ${b}` : "";
  }
  function findMatchup() {
    let m = matchupFrom(raw);
    if (m) return m;
    for (const t of metaTexts) { m = matchupFrom(t); if (m) return m; }
    for (const t of selectedTexts) { m = matchupFrom(t); if (m) return m; }
    return "";
  }
  function matchupTeams(match) {
    const m = String(match || "").match(/^(.+?)\s+vs\s+(.+)$/i);
    return m ? [m[1].trim(), m[2].trim()] : [];
  }
  function escapeRegexLiteral(value) {
    return String(value || "").replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  }
  function normalizeUnit(text) {
    const official = globalThis.BettyMarketTitles?.unit(text, marketSlug);
    if (official) return official;
    const x = String(text || "").toLowerCase();
    if (/\b(corner|corners|corner kicks|escanteios)\b/.test(x)) return "escanteios";
    if (/\b(goal|goals|gols)\b/.test(x)) return "gols";
    if (/\b(run|runs)\b|baseball|mlb/.test(x)) return "runs";
    if (/\b(point|points)\b/.test(x)) return "pontos";
    return "";
  }
  function capOU(v) { const x=String(v||"").toLowerCase(); return x ? x[0].toUpperCase()+x.slice(1) : x; }
  function extractLine(texts) {
    for (const t of texts) {
      const x=String(t||"");
      let m=x.match(/\b(?:over|under)\s+([0-9]+(?:[.,][0-9]+)?)/i);
      if (m) return m[1].replace(",", ".");
      m=x.match(/\b(?:O\/U|Over\/Under|Total)\s*([0-9]+(?:[.,][0-9]+)?)/i);
      if (m) return m[1].replace(",", ".");
    }
    return "";
  }
  function extractOU(texts) {
    // The held position outcome is the source of truth. An O/U label in a
    // group title must never override the actual outcome (Under/Over).
    const outcomeMatch = outcomeRaw.match(/\b(over|under)\b/i);
    if (outcomeMatch) return outcomeMatch[1].toLowerCase();
    const sideMatch = side.match(/\b(over|under)\b/i);
    if (sideMatch) return sideMatch[1].toLowerCase();
    return "";
  }
  function extractPeriod(texts, typeHint) {
    const official = globalThis.BettyMarketTitles?.period([typeHint, ...texts].join(' '));
    if (official) return official;
    const joined=texts.map(v=>String(v||"")).join(" ");
    const type=String(typeHint||"").toLowerCase();
    // Baseball period belongs only to the exact market metadata.
    if (/(?:first|1st)[_\s-]*(?:5|five)[_\s-]*innings?/.test(type) || /\b(?:first|1st)\s*(?:5|five)\s+innings?\b/i.test(joined) || /\b1st\s*5\s+innings?\b/i.test(joined)) return "1st 5 Innings";
    if (/(?:first|1st)[_\s-]*half/.test(type)) return "1st Half";
    if (/(?:second|2nd)[_\s-]*half/.test(type)) return "2nd Half";
    if (/\b(?:first|1st)\s+half\b/i.test(joined) || /\b1H\b/i.test(joined)) return "1st Half";
    if (/\b(?:second|2nd)\s+half\b/i.test(joined) || /\b2H\b/i.test(joined)) return "2nd Half";
    return "";
  }
  function isGenericGroup(g) {
    return !g || /^(?:yes|no|over|under|draw|o\/u|over\/under|total)(?:\s+[-+]?\d+(?:[.,]\d+)?)?$/i.test(g);
  }
  function findSpecificTeam(texts, match, groupTitle) {
    const teams=matchupTeams(match);
    if (teams.length !== 2) return "";
    const sources=texts.map(v=>String(v||"")).filter(Boolean);
    // groupItemTitle can be the exact team label for secondary/team-total markets.
    if (!isGenericGroup(groupTitle)) {
      const g=cleanTeam(groupTitle);
      const hit=teams.find(t=>t.toLowerCase()===g.toLowerCase() || g.toLowerCase().startsWith(t.toLowerCase()+" "));
      if (hit) return hit;
    }
    for (const text of sources) {
      const lower=text.toLowerCase();
      // Explicit team-total questions: "Will TEAM score/have ..."
      const m=text.match(/^\s*(?:will|does|can)\s+(.+?)\s+(?:score|have|get|record|make|take)\b/i);
      if (m) {
        const candidate=m[1].trim();
        const hit=teams.find(t=>candidate.toLowerCase()===t.toLowerCase() || candidate.toLowerCase().includes(t.toLowerCase()) || t.toLowerCase().includes(candidate.toLowerCase()));
        if (hit) return hit;
      }
      // Group/title forms such as "TEAM O/U 2.5" or "TEAM - O/U 2.5".
      const marketLabel=text.split(/:\s*/).pop().trim();
      for (const t of teams) {
        if (new RegExp("^\\s*" + escapeRegexLiteral(t) + "\\s*(?:[-–—:]\\s*)?(?:O\\/U|Over\\/Under)\\b", "i").test(marketLabel)) return t;
      }
      // If exactly one team is named in the exact market question, it is a team market.
      const hits=teams.filter(t=>t && lower.includes(t.toLowerCase()));
      if (hits.length===1 && /\b(?:score|have|get|record|make|take)\b/i.test(text)) return hits[0];
    }
    return "";
  }
  // Baseball team totals: identify the team only from the exact market metadata.
  function findBaseballTeam(texts, match, groupTitle) {
    const teams=matchupTeams(match);
    if (teams.length!==2) return "";
    const norm=v=>String(v||"").replace(//g," ").trim().toLowerCase();
    const generic=/^(?:yes|no|over|under|draw|o\/u|over\/under|total|spread|winner|moneyline|ml)(?:\s+[-+]?\d+(?:[.,]\d+)?)?$/i;
    const g=cleanTeam(groupTitle);
    if (g && !generic.test(g)) {
      const hit=teams.find(t=>norm(t)===norm(g) || norm(g).startsWith(norm(t)+" "));
      if (hit) return hit;
    }
    for (const text of texts.map(v=>String(v||"").replace(/\s+/g," ").trim()).filter(Boolean)) {
      const m=text.match(/^\s*(?:will|does|can)\s+(.+?)\s+(?:score|have|get|record|make|take)\b/i);
      if (m) {
        const candidate=cleanTeam(m[1]);
        const hit=teams.find(t=>norm(candidate)===norm(t) || norm(candidate).includes(norm(t)) || norm(t).includes(norm(candidate)));
        if (hit) return hit;
      }
      for (const t of teams) {
        const esc=escapeRegexLiteral(t);
        if (new RegExp("^\\s*"+esc+"\\s*(?:[-–—:]\\s*)?(?:O\\/U|Over\\/Under)\\b","i").test(text)) return t;
      }
    }
    return "";
  }


  let matchup = "";
  const preMatchupQuestion = String(selectedMarket?.question || "").replace(/\s+/g, " ").trim();
  if (/\bwill\s+the\s+game\s+go\s+to\s+extra\s+innings\b/i.test(preMatchupQuestion)) {
    const afterColon = preMatchupQuestion.split(/:\s*/).slice(1).join(": " ).trim();
    if (afterColon) matchup = matchupFrom(afterColon);
  }
  if (!matchup) matchup = findMatchup();
  const extendedTitle = globalThis.BettyMarketTitles?.extendedTitle(d, matchup);
  if (extendedTitle) return extendedTitle;
  if (!matchup) return raw;

  // Os slugs-base de ATP/WTA/ITF representam somente o vencedor da partida.
  // Alguns registros legados não têm mais a resposta Gamma persistida, mas
  // ainda possuem o confronto e o participante escolhido. Mantemos este
  // fallback deliberadamente estrito para não reinterpretar outros mercados.
  const legacyTennisMoneyline = !d?._sportMeta
    && /^(?:atp|wta|itf)(?:-doubles)?-[a-z0-9]+-[a-z0-9]+-\d{4}-\d{2}-\d{2}$/i.test(marketSlug);
  if (legacyTennisMoneyline) {
    const source = raw.includes(":") ? raw.slice(raw.lastIndexOf(":") + 1).trim() : raw;
    const legacyMatch = source.match(/^(.+?)\s+vs\.?\s+(.+)$/i);
    if (legacyMatch) {
      const home = cleanTeam(legacyMatch[1]);
      const away = cleanTeam(legacyMatch[2]);
      const target = cleanTeam(outcomeRaw);
      const norm = value => String(value || "").trim().toLocaleLowerCase("en-US");
      if (target && (norm(target) === norm(home) || norm(target) === norm(away))) {
        return `${home} vs ${away} - ${target} ML`;
      }
    }
  }

  // A base sports slug, bare matchup and exact participant outcome are
  // required before inferring a legacy full-game winner contract.
  if (!selectedMarket && /^(?:cfb|nfl|nba|wnba|nhl|bkbsl)-[a-z0-9]+-[a-z0-9]+-\d{4}-\d{2}-\d{2}$/i.test(marketSlug)) {
    const bare = raw.match(/^(.+?)\s+vs\.?\s+([^:]+)$/i);
    if (bare && [bare[1], bare[2]].some(t => t.trim().toLowerCase() === outcomeRaw.toLowerCase())) return `${matchup} - ${outcomeRaw} ML`;
  }

  // Defesa somente para o caso em que a Gamma esteja realmente indisponível.
  // Não concorre com selectedMarket: quando há _sportMeta, a normalização
  // oficial acima continua sendo a única fonte para linha, período e time.
  if (!d?._sportMeta) {
    const cornerMatch = raw.match(/\b(?:O\/U|Over\/Under|Total)\s*([0-9]+(?:[.,][0-9]+)?)\s*(?:Total\s*)?(?:Corners?|Corner\s+Kicks?)\b/i);
    const cornerOutcome = outcomeRaw.match(/\b(over|under)\b/i);
    if (cornerMatch && cornerOutcome) {
      const line = cornerMatch[1].replace(",", ".");
      const prefix = raw.slice(0, cornerMatch.index).split(/:\s*/).pop().trim();
      const teams = matchupTeams(matchup);
      const team = teams.find(t => prefix.toLowerCase().startsWith(t.toLowerCase())) || "";
      const period = extractPeriod([prefix], "");
      const ou = cornerOutcome[1][0].toUpperCase() + cornerOutcome[1].slice(1).toLowerCase();
      return `${matchup} - ${team ? `${team} ` : ""}${period ? period + ' ' : ''}${ou} ${line} escanteios`;
    }
    // Totais legados sem meta Gamma: o título bruto ainda contém confronto,
    // lado e linha suficientes. Este ramo é deliberadamente estrito para não
    // reinterpretar outros mercados que devam permanecer no fallback genérico.
    const totalMatch = raw.match(/\b(?:O\/U|Over\/Under|Total)\s*([0-9]+(?:[.,][0-9]+)?)\b/i);
    const totalOutcome = outcomeRaw.match(/\b(over|under)\b/i);
    if (totalMatch && totalOutcome) {
      const prefix = raw.slice(0, totalMatch.index).split(/:\s*/).pop().trim();
      const teams = matchupTeams(matchup);
      const team = teams.find(t => prefix.toLowerCase().startsWith(t.toLowerCase())) || "";
      const hint = [d?.eventSlug, d?.slug, d?.sport, d?.category].filter(Boolean).join(" ").toLowerCase();
      const classifiedUnit = typeof classificarAposta === 'function'
        ? globalThis.BettyMarketTitles?.sportUnit(classificarAposta(d)) : '';
      const unit = globalThis.BettyMarketTitles?.unit(raw, marketSlug) || classifiedUnit || (/(?:^|[-_\s])(?:cfb|nfl|ncaaf|cfl|ufl|xfl)(?:[-_\s]|$)|\b(?:american football|points?)\b/.test(hint)
        ? "pontos" : /\b(?:mlb|npb|kbo|baseball|runs?)\b/.test(hint) ? "runs" : "gols");
      const ou = totalOutcome[1][0].toUpperCase() + totalOutcome[1].slice(1).toLowerCase();
      const period = extractPeriod([prefix], "");
      return `${matchup} - ${team ? `${team} ` : ""}${period ? period + ' ' : ''}${ou} ${totalMatch[1].replace(",", ".")} ${unit}`;
    }
  }

  // BTTS não é Over/Under: o resultado Yes/No e o período vêm somente do
  // mercado exato, evitando que mercados irmãos do mesmo evento contaminem o
  // rótulo. Sem meta oficial, preservamos o fallback textual original.
  if (selectedMarket) {
    const bttsText = [marketQuestion, ...selectedTexts].join(" ");
    if (/\bboth\s+teams\s+to\s+score\b/i.test(bttsText)) {
      const yn = outcomeRaw.match(/^(Yes|No)$/i);
      if (yn) {
        const firstHalf = /\b(?:first|1st)\s+half\b/i.test(bttsText) ? " 1st Half" : "";
        return `${matchup} - Both Teams to Score${firstHalf} ${yn[1][0].toUpperCase()}${yn[1].slice(1).toLowerCase()}`;
      }
    }
  }

  // Tênis usa unidades próprias. Estes ramos só aceitam o mercado exato
  // encontrado por asset/conditionId/slug; assim um mercado irmão jamais
  // troca o período, a linha ou o lado da posição exibida.
  if (selectedMarket) {
    const tennisMatch = marketQuestion.match(/^(.+?)\s+vs\.?\s+(.+?)\s*:\s*(?:Total\s+Sets|Match\s+O\/U|Set\s+1\s+Games\s+O\/U)\b/i);
    const tennisMatchup = tennisMatch ? `${cleanTeam(tennisMatch[1])} vs ${cleanTeam(tennisMatch[2])}` : matchup;
    const tennisOu = extractOU([marketQuestion]);
    const tennisLine = extractLine([marketQuestion]) || (selectedMarket?.line != null ? String(selectedMarket.line).replace(",", ".") : "");

    if (marketType === "tennis_set_totals" && tennisOu && tennisLine) {
      return `${tennisMatchup} - ${capOU(tennisOu)} ${tennisLine} sets`;
    }
    if (marketType === "tennis_match_totals" && tennisOu && tennisLine) {
      return `${tennisMatchup} - ${capOU(tennisOu)} ${tennisLine} games`;
    }
    if (marketType === "tennis_first_set_totals" && tennisOu && tennisLine) {
      return `${tennisMatchup} - 1st Set: ${capOU(tennisOu)} ${tennisLine} games`;
    }
    if (marketType === "tennis_set_handicap") {
      const handicap = marketQuestion.match(/^Set\s+Handicap:\s*(.+?)\s*\(\s*([+-]\d+(?:[.,]\d+)?)\s*\)\s+vs\.?\s+(.+?)\s*\(\s*([+-]\d+(?:[.,]\d+)?)\s*\)\s*$/i);
      if (handicap) {
        const home = cleanTeam(handicap[1]);
        const away = cleanTeam(handicap[3]);
        const target = cleanTeam(outcomeRaw);
        const norm = value => String(value || "").trim().toLocaleLowerCase("en-US");
        const line = norm(target) === norm(home) ? handicap[2] : norm(target) === norm(away) ? handicap[4] : "";
        if (line) return `${home} vs ${away} - Handicap de sets: ${target} ${line.replace(",", ".")}`;
      }
    }
  }

  if (selectedMarket && marketType === 'rebounds') {
    const prop = marketQuestion.match(/^(.+?):\s*Rebounds\s+O\/U\s+(\d+(?:[.,]\d+)?)$/i);
    if (prop) {
      const ou = extractOU([]);
      const selection = ou ? `${capOU(ou)} ${prop[2]} rebotes` : `Rebotes O/U ${prop[2]} ${outcomeRaw}`;
      return `${matchup} - ${prop[1].trim()} ${selection}`;
    }
  }
  if (selectedMarket && marketType === 'map_handicap') {
    const handicap = marketQuestion.match(/^(Map|Game)\s+Handicap:\s*(.+?)\s*\(([+-]\d+(?:[.,]\d+)?)\)\s+vs\.?\s+(.+?)\s*\(([+-]\d+(?:[.,]\d+)?)\)$/i);
    if (handicap) {
      // Alias resolution must be unique; otherwise preserve the signed
      // official question and show the held selection without guessing.
      const norm = value => String(value).normalize('NFKC').replace(/[\u200B-\u200F\u2060\uFEFF]/g, '').toLowerCase().replace(/[^\p{L}\p{N}]/gu, '');
      const target = norm(outcomeRaw), a = norm(handicap[2]), b = norm(handicap[4]);
      const hits = [a, b].map(t => t === target || (t.length >= 3 && target.startsWith(t)));
      if (hits.filter(Boolean).length === 1) {
        const line = hits[0] ? handicap[3] : handicap[5];
        return `${cleanTeam(handicap[2])} vs ${cleanTeam(handicap[4])} - Handicap de ${handicap[1].toLowerCase() === 'map' ? 'mapas' : 'games'}: ${outcomeRaw} ${line}`;
      }
      return `${marketQuestion} - Seleção: ${outcomeRaw}`;
    }
  }

  // Detecta cedo se esta é uma posição de Spread/Handicap usando SOMENTE o
  // mercado exato da posição (selectedTexts) + o título bruto + o outcome/side
  // da própria posição. Nunca usa metaTexts (agregado de TODOS os mercados do
  // evento), pois isso contamina o resultado com texto de mercados irmãos
  // (ex.: o mercado de vencedor do mesmo evento) e é a causa raiz de posições
  // de spread caindo indevidamente na label "ML".
  const spreadSources = [...selectedTexts, raw].filter(Boolean);
  const spreadText = spreadSources.join(" | ");
  const rawSpreadLineHint = /(?:^|\s)(?:[+-]\d+(?:[.,]\d+)?)\b/.test(raw)
    || /(?:^|\s)(?:[+-]\d+(?:[.,]\d+)?)\b/.test(outcomeRaw)
    || /(?:^|\s)(?:[+-]\d+(?:[.,]\d+)?)\b/.test(side);
  const isSpreadMarket = /\b(?:spread|handicap)\b/i.test(spreadText)
    || /^(?:spread|handicap)$/i.test(marketType)
    || rawSpreadLineHint;

  // Mercados binários especiais da Polymarket: estes formatos têm uma
  // pergunta de resolução própria e não devem cair no normalizador genérico.
  // A decisão usa SOMENTE o mercado exato da posição + o outcome da posição.
  const exactQuestion = String(selectedMarket?.question || "").replace(/\s+/g, " ").trim();
  const exactResolutionText = [exactQuestion, ...selectedTexts].join(" ");
  if (/\bextra\s+innings\b/i.test(exactResolutionText) && /\bwill\s+the\s+game\s+go\s+to\s+extra\s+innings\b/i.test(exactResolutionText)) {
    const yn = outcomeRaw.match(/^(Yes|No)$/i);
    if (yn) return `${matchup} - Extra Innings ${yn[1][0].toUpperCase()}${yn[1].slice(1).toLowerCase()}`;
  }
  // Nunca gerar label "ML" para uma posição já identificada como Spread/Handicap.
  if (!isSpreadMarket && marketType !== "moneyline" && /\bresolve\s+to\b/i.test(exactResolutionText) && /\bif\b/i.test(exactResolutionText)) {
    const target = cleanTeam(outcomeRaw);
    if (target && !/^(?:yes|no|over|under|draw)$/i.test(target)) {
      return `${matchup} - ${target} ML`;
    }
  }

  // Moneyline normal: o tipo do mercado exato e o outcome da posição são
  // suficientes. Não dependemos da description (omitida intencionalmente do
  // cache compacto), nem do texto bruto "vs.", que não identifica o lado.
  // Este ramo vem após os binários especiais acima e antes do fallback
  // genérico para preservar seus rótulos próprios.
  if (selectedMarket && marketType === "moneyline") {
    // Usa o outcome original: `side` remove o prefixo "No" para mercados
    // binários e corromperia times legítimos como "Notre Dame".
    const target = cleanTeam(outcomeRaw);
    if (target && !/^(?:yes|no|over|under)$/i.test(target)) {
      // Para tênis, a pergunta oficial começa com o torneio. O título do
      // mercado deve mostrar somente o confronto, como os demais mercados
      // de tênis já normalizados, sem perder o atleta/par selecionado.
      const isTennisMoneyline = /^(?:atp|wta|itf)(?:-doubles)?-/i.test(marketSlug)
        || /\btennis\b/i.test([d?.sport, d?.category, selectedMarket?.category].filter(Boolean).join(" "));
      const questionMatch = isTennisMoneyline
        ? (marketQuestion.includes(":") ? marketQuestion.slice(marketQuestion.lastIndexOf(":") + 1).trim() : marketQuestion)
          .match(/^(.+?)\s+vs\.?\s+(.+)$/i)
        : null;
      const displayMatchup = questionMatch
        ? `${cleanTeam(questionMatch[1])} vs ${cleanTeam(questionMatch[2])}`
        : matchup;
      const americanFootball = /^(?:cfb|ncaaf|nfl|cfl|ufl|xfl)-/i.test(marketSlug)
        || (typeof classificarAposta === 'function' && classificarAposta(d) === 'Futebol Americano');
      return americanFootball
        ? `${displayMatchup} - ${globalThis.BettyMarketTitles.footballWinner(target, selectedTexts.join(' '))}`
        : `${displayMatchup} - ${target} ML`;
    }
  }

  // With exact market metadata, NEVER use raw title to infer period, team, line or unit.
  // This is the key guard against one market in an event contaminating another.
  const leadQs=[marketQuestion,...selectedTexts].filter(Boolean);
  for(const q of leadQs){
    const m=String(q).replace(/\s+/g," ").trim().match(/^(?:Will\s+)?(.+?)\s+(?:be\s+)?leading\s+at\s+halftime\??$/i);
    if(m&&/^(?:yes|no)$/i.test(outcomeRaw.trim())){
      const team=cleanTeam(m[1]);
      if(team)return `${matchup} - 1st Half ${team} ML${outcomeRaw.trim().toLowerCase() === 'no' ? ' No' : ''}`;
    }
  }
  const exact=selectedMarket ? selectedTexts : [];
  const analysis=exact.join(" ");
  const baseballBinary = selectedMarket && /^(?:mlb|npb|kbo|cpbl|milb)-/i.test(marketSlug) && /^(?:yes|no)$/i.test(outcomeRaw);
  if (baseballBinary && /inning/i.test(marketQuestion) && /\btie(?:s)?\b|First Inning Run\??$/i.test(marketQuestion)) {
    const detail = globalThis.BettyMarketTitles.kalshiDetail({title:marketQuestion},outcomeRaw.toLowerCase(),marketType);
    if(detail)return `${matchup} - ${detail}`;
  }
  const footballMarket = selectedMarket && (/^(?:cfb|ncaaf|nfl|cfl|ufl|xfl)-/i.test(marketSlug)
    || (typeof classificarAposta === 'function' && classificarAposta(d) === 'Futebol Americano'));
  const footballWinnerQuestion = footballMarket && marketQuestion.match(/^(?:Will\s+)?(.+?)\s+win(?:s)?(?:\s+the)?\s+((?:1st|2nd|first|second)\s+half|[1-4](?:st|nd|rd|th)\s+quarter)\??$/i);
  if (footballWinnerQuestion && /^(?:yes|no)$/i.test(outcomeRaw)) {
    return `${matchup} - ${globalThis.BettyMarketTitles.footballWinner(cleanTeam(footballWinnerQuestion[1]), footballWinnerQuestion[2], outcomeRaw.toLowerCase())}`;
  }
  const ou=extractOU(exact);
  const linha=extractLine(exact);
  // Explicit market units always win (corners, rebounds, etc.). Only use the
  // classified sport for an otherwise unspecified total; never sibling titles.
  const sportUnit = selectedMarket && typeof classificarAposta === 'function'
    ? globalThis.BettyMarketTitles?.sportUnit(classificarAposta(d)) : '';
  const unit=normalizeUnit(analysis) || (selectedMarket ? normalizeUnit(selectedMarket?.category || "") : "") || sportUnit || "gols";
  const period=extractPeriod(exact, marketType);
  const isBaseball = /\b(?:baseball|mlb|npb|kbo|cpbl|milb)\b/i.test(analysis) || /\b(?:innings?|runs?)\b/i.test(analysis);
  const team=isBaseball ? (findBaseballTeam(exact, matchup, groupItemTitle) || findSpecificTeam(exact, matchup, groupItemTitle)) : findSpecificTeam(exact, matchup, groupItemTitle);

  if (ou && linha) {
    if (team && period) return `${matchup} - ${team} ${period} ${capOU(ou)} ${linha} ${unit}`;
    if (team) return `${matchup} - ${team} ${capOU(ou)} ${linha} ${unit}`;
    if (period) return `${matchup} - ${period} ${capOU(ou)} ${linha} ${unit}`;
    return `${matchup} - ${capOU(ou)} ${linha} ${unit}`;
  }

  // Spread/handicap: o mercado exato define time, linha e período.
  // Nunca tratar Spread como ML e nunca assumir uma linha fixa.
  // (spreadSources/spreadText/isSpreadMarket já calculados acima, cedo,
  // para poder guardar os ramos de ML antes deles também.)
  if (isSpreadMarket) {
    const teams = matchupTeams(matchup);
    let spreadLine = "";
    let spreadPeriod = extractPeriod(spreadSources, marketType);

    // O time exibido no rótulo é SEMPRE o time do outcome da posição —
    // nunca o time citado no texto do mercado, que às vezes descreve o
    // spread pela perspectiva do adversário (ex.: mercado fala "Halmstads
    // BK -1.5" mas a posição comprada foi em Orgryte IS).
    const cleanOutcomeTeam = stripMarketSuffix(side || "");
    const spreadTeam = teams.find(t => t.toLowerCase() === cleanOutcomeTeam.toLowerCase() || cleanOutcomeTeam.toLowerCase().includes(t.toLowerCase()) || t.toLowerCase().includes(cleanOutcomeTeam.toLowerCase())) || cleanOutcomeTeam;

    // parsedTeam é apenas o time a que o TEXTO da linha se refere — usado
    // só para saber se a linha precisa ser invertida de sinal quando o
    // texto descreve o spread pela perspectiva do adversário do outcome.
    let parsedTeam = "";

    // Formatos comuns: "Spread: Team (+1.5)", "1st Half Spread: Team (-0.5)"
    // e "1st 5 Innings Spread: Team (+1.5)".
    const spreadPatterns = [
      /(?:1st\s*5|first\s*5)\s+innings?\s+spread\s*[:\-–—]?\s*(.+?)\s*\(\s*([+-]?\d+(?:[.,]\d+)?)\s*\)/i,
      /(?:1st\s+half|first\s+half)\s+spread\s*[:\-–—]?\s*(.+?)\s*\(\s*([+-]?\d+(?:[.,]\d+)?)\s*\)/i,
      /(?:spread|handicap)\s*[:\-–—]?\s*(.+?)\s*\(\s*([+-]?\d+(?:[.,]\d+)?)\s*\)/i,
      /(?:1st\s*5|first\s*5)\s+innings?\s+spread\s*[:\-–—]?\s*(.+?)\s+([+-]?\d+(?:[.,]\d+)?)(?:\s|$)/i,
      /(?:1st\s+half|first\s+half)\s+spread\s*[:\-–—]?\s*(.+?)\s+([+-]?\d+(?:[.,]\d+)?)(?:\s|$)/i,
      /(?:spread|handicap)\s*[:\-–—]?\s*(.+?)\s+([+-]?\d+(?:[.,]\d+)?)(?:\s|$)/i
    ];
    for (const text of spreadSources) {
      for (const re of spreadPatterns) {
        const m = String(text).match(re);
        if (!m) continue;
        const candidate = stripMarketSuffix(m[1]);
        const hit = teams.find(t => t.toLowerCase() === candidate.toLowerCase() || candidate.toLowerCase().includes(t.toLowerCase()) || t.toLowerCase().includes(candidate.toLowerCase()));
        if (hit) parsedTeam = hit;
        if (m[2]) spreadLine = String(m[2]).replace(",", ".");
        break;
      }
      if (parsedTeam && spreadLine) break;
    }

    // Alguns mercados usam groupItemTitle como "Team +1.5" e question/type
    // apenas como "Spread". Nesse caso o groupItemTitle continua sendo a fonte
    // do lado e da linha.
    const groupSpread = String(selectedMarket?.groupItemTitle || "").match(/^(.+?)\s+([+-]\d+(?:[.,]\d+)?)$/);
    if (groupSpread) {
      const candidate = stripMarketSuffix(groupSpread[1]);
      const hit = teams.find(t => t.toLowerCase() === candidate.toLowerCase() || candidate.toLowerCase().includes(t.toLowerCase()) || t.toLowerCase().includes(candidate.toLowerCase()));
      if (hit) parsedTeam = hit;
      spreadLine = groupSpread[2].replace(",", ".");
    }

    if (!spreadLine && selectedMarket?.line != null) {
      const metaLine = String(selectedMarket.line).match(/[+-]?\d+(?:[.,]\d+)?/);
      if (metaLine) spreadLine = metaLine[0].replace(",", ".");
    }
    if (!spreadLine) {
      const lineMatch = spreadText.match(/([+-]\d+(?:[.,]\d+)?)/);
      if (lineMatch) spreadLine = lineMatch[1].replace(",", ".");
    }

    if (spreadTeam && spreadLine) {
      // Se a linha foi extraída de um texto que descreve o time ADVERSÁRIO
      // do outcome, o spread é o inverso (a linha de um time é sempre o
      // negativo da linha do outro). Se não sabemos a quem o texto se
      // referia, usamos a linha como veio, sem inverter.
      let lineNum = Number(spreadLine);
      if (parsedTeam && spreadTeam && parsedTeam.toLowerCase() !== spreadTeam.toLowerCase() && Number.isFinite(lineNum)) {
        lineNum = -lineNum;
      }
      const lineText = Number.isFinite(lineNum) ? `${lineNum > 0 ? "+" : ""}${lineNum}` : `${spreadLine.startsWith("+") || spreadLine.startsWith("-") ? "" : (Number(spreadLine) > 0 ? "+" : "")}${spreadLine}`;
      const target = spreadPeriod ? `${spreadTeam} ${spreadPeriod}` : spreadTeam;
      return `${matchup} - ${target} ${lineText}`;
    }
  }

  if (/\bto advance\b|\badvances?\b/i.test(raw) || /\badvance/i.test(side)) {
    let team=side.replace(/\s+(?:advances?|to advance)$/i, "").trim();
    if (!team) team=raw.replace(/.*?:\s*/, "").replace(/\s+to advance\??$/i, "").trim();
    return `${matchup} - ${team||side} Advances`;
  }
  // metaTexts é o agregado de TODOS os mercados do evento (inclusive mercados
  // irmãos, como o de vencedor da partida). Nunca usar metaTexts aqui: isso
  // contaminava posições de Spread/Handicap com a pergunta "Will X win" do
  // mercado de vencedor do mesmo evento, virando "ML Time" errado. Detecção
  // de vencedor usa somente o mercado exato da posição (raw + exact) e nunca
  // dispara para uma posição já identificada como Spread/Handicap.
  const winnerQuestion = !isSpreadMarket ? [raw,...exact].find(t=>/\bwill\s+.+?\s+win\b/i.test(String(t||""))) : null;
  if (winnerQuestion) {
    const wm=String(winnerQuestion).match(/\bwill\s+(.+?)\s+win\b/i);
    if (wm && stripMarketSuffix(wm[1])) return `${matchup} - ${stripMarketSuffix(wm[1])} ML`;
  }
  const mm=raw.match(/\bMore\s+Markets\b\s*[-–—:]\s*(.+)$/i);
  if (mm) return `${matchup} - ${mm[1].replace(/\?+$/," ").trim()}`.replace(/\s+$/," ");
  if (!isSpreadMarket && (/\b(?:winner|to win|moneyline)\b|^ML\b/i.test(raw) || /\b(?:to win|wins?)\b/i.test(side))) {
    const team=side.replace(/\s+(?:to win|wins?|is the winner)$/i, "").trim();
    if (team) return `${matchup} - ${team} ML`;
  }
  // O matchup mostrado troca "vs." por "vs". Nunca o use como índice no
  // título bruto, pois indexOf(-1) era a origem de fragmentos como "ub:".
  const trechoComConfronto = raw.match(/^[\s\S]*?\s+(?:vs\.?|versus|@)\s+[\s\S]*?:\s*/i);
  const indiceConfronto = raw.toLowerCase().indexOf(matchup.toLowerCase());
  if (!trechoComConfronto && indiceConfronto < 0) return raw;
  const inicioComplemento = trechoComConfronto ? trechoComConfronto[0].length : indiceConfronto + matchup.length;
  let complemento=raw.slice(inicioComplemento).replace(/^\s*[:\-–—]\s*/,"").trim();
  complemento=complemento.replace(/^More\s+Markets\s*(?:[:\-–—]\s*)?/i,"").replace(/^\s*[-–—]\s*/,"").replace(/\?+$/," ").trim();
  complemento=complemento.replace(/\bMore\s+Markets\b\s*[-–—:]?\s*/gi,"").replace(/\s{2,}/g," ").trim();
  return complemento && !/^winner$/i.test(complemento) ? `${matchup} - ${complemento}` : matchup;
}

// Normalização de nomes de mercados da Kalshi para o mesmo padrão usado na
// Polymarket. Só reconhece os formatos abaixo (exatamente os já validados
// com o usuário); qualquer coisa fora disso devolve o título original sem
// alteração, para nunca arriscar exibir algo errado.
function normalizarNomeMercadoKalshi(d) {
  if (!d || String(d._platform || "").toLowerCase() !== "kalshi") return d?.title || "";
  const specialTitle = globalThis.BettyMarketTitles?.kalshiSpecialTitle(d);
  if (specialTitle) return specialTitle;
  const inningTitle = globalThis.BettyMarketTitles?.inningWinnerTitle(d);
  if (inningTitle) return inningTitle;
  if (d._kalshiTitleMeta && globalThis.BettyMarketTitles) {
    const meta = d._kalshiTitleMeta;
    const americanFootball = /^KX(?:NCAAF|NFL|CFL|UFL|XFL)/i.test(d.ticker || d.market_ticker || d.slug || '')
      || (typeof classificarAposta === 'function' && classificarAposta(d) === 'Futebol Americano');
    const detail = globalThis.BettyMarketTitles.kalshiDetail(meta.market, String(d.outcome || '').toLowerCase(), meta.scope, americanFootball);
    const officialMatchup = String(meta.matchup || '').replace(/\bvs\./g, 'vs');
    const rawMatchup = String(d.title || '').split(' - ')[0].replace(/\bvs\./g, 'vs');
    const officialTeams = officialMatchup.split(/\s+vs\s+/i);
    const existingTeams = rawMatchup.split(/\s+vs\s+/i);
    // Keep the established spelling/order. Repair a clipped prefix only
    // when the official participant uniquely proves the missing characters.
    const restoredTeams = existingTeams.map(team => {
      const candidates = officialTeams.filter(t => t.length > team.length && t.length - team.length <= 3 && t.toLowerCase().endsWith(team.toLowerCase()));
      return candidates.length === 1 ? candidates[0] : team;
    });
    const matchup = existingTeams.length === 2 ? restoredTeams.join(' vs ') : officialMatchup;
    if (detail && matchup) return `${matchup} - ${detail}`;
    const winner = String(meta.market?.title || '').match(/^(.+?)\s+wins?\??$/i);
    if (winner && matchup) return `${matchup} - ${winner[1]} ML${String(d.outcome).toLowerCase() === 'no' ? ' No' : ''}`;
  }
  const raw = String(d.title || "").replace(/\s+/g, " ").trim();
  if (!raw) return "";

  // Alguns eventos da Kalshi não trazem o nome dos times em NENHUM campo
  // (nem nos IDs estruturados do evento, nem em texto livre) — a pergunta
  // do mercado já chega assim da própria Kalshi, sem confronto algum. Não
  // há como recuperar os times nesse caso; ao menos aplicamos o mesmo
  // formato Over/Under usado no resto do app, em vez de deixar a pergunta
  // bruta na tela.
  const bareOU = raw.match(/^Will\s+(over|under)\s+([0-9]+(?:[.,]\d+)?)\s+(goals?|runs?|points?)\s+be\s+scored\??$/i);
  if (bareOU) {
    const ou = bareOU[1][0].toUpperCase() + bareOU[1].slice(1).toLowerCase();
    const linha = bareOU[2].replace(",", ".");
    const x = bareOU[3].toLowerCase();
    const unidade = /^run/.test(x) ? "runs" : /^point/.test(x) ? "pontos" : "gols";
    return `${ou} ${linha} ${unidade}`;
  }

  const dashIdx = raw.indexOf(" - ");
  const matchupPart = dashIdx >= 0 ? raw.slice(0, dashIdx).trim() : raw;
  const complemento = dashIdx >= 0 ? raw.slice(dashIdx + 3).trim() : "";
  if (!complemento) return raw;

  const vsMatch = matchupPart.match(/^(.+?)\s+vs\.?\s+(.+)$/i);
  if (!vsMatch) return raw;
  const teams = [vsMatch[1].trim(), vsMatch[2].trim()];
  const matchup = `${teams[0]} vs ${teams[1]}`;

  const outcomeRaw = String(d.outcome || "").replace(/\s+/g, " ").trim() || "Yes";

  const explicitSide=complemento.match(/:\s*(Yes|No)$/i);
  const compactQuestion=explicitSide && explicitSide[1].toLowerCase()===outcomeRaw.toLowerCase()
    ? complemento.slice(0,explicitSide.index).trim() : complemento;
  const compactTotal=Number(d._kalshiTitleFormatVersion||0)<4
    ? globalThis.BettyMarketTitles?.compactKalshiTotal(compactQuestion,outcomeRaw) : '';
  if(compactTotal)return `${matchup} - ${compactTotal}`;

  function findTeam(candidate) {
    const c = String(candidate || "").trim();
    if (!c) return "";
    return teams.find(t => t.toLowerCase() === c.toLowerCase() || t.toLowerCase().includes(c.toLowerCase()) || c.toLowerCase().includes(t.toLowerCase())) || "";
  }

  // "{Time} wins 1st/2nd Half" -> "{Time} 1st/2nd Half Yes/No"
  let m = complemento.match(/^(.+?)\s+wins?\s+(1st|first|2nd|second)\s+half$/i);
  if (m) {
    const team = findTeam(m[1]) || m[1].trim();
    const period = /^(?:1st|first)$/i.test(m[2]) ? "1st Half" : "2nd Half";
    return `${matchup} - ${team} ${period} ${outcomeRaw}`;
  }

  // "{Time} advances" -> nome completo do time, igual ao usado no confronto.
  m = complemento.match(/^(.+?)\s+advances?$/i);
  if (m) {
    const team = findTeam(m[1]) || m[1].trim();
    return `${matchup} - ${team} Advances`;
  }

  // "Reg Time: {Time}" / "Regulation Time: {Time}" -> vencedor no tempo
  // regulamentar (empate/prorrogação à parte) -> mesmo rótulo "ML" usado
  // para vencedor na Polymarket.
  m = complemento.match(/^Reg(?:ulation)?\s*Time\s*[:\-–—]\s*(.+)$/i);
  if (m) {
    const team = findTeam(m[1]) || m[1].trim();
    return `${matchup} - ${team} ML${outcomeRaw.toLowerCase() === 'no' ? ' No' : ''}`;
  }

  function unitFromWord(w) {
    const x = String(w || "").toLowerCase();
    if (/^run/.test(x)) return "runs";
    if (/^goal/.test(x)) return "gols";
    if (/^point/.test(x)) return "pontos";
    return "";
  }

  // "Over/Under N {unidade} in the first 5 innings" -> total do jogo no
  // período "1st 5 Innings", igual ao formato de período da Polymarket.
  m = complemento.match(/^(over|under)\s+([0-9]+(?:[.,]\d+)?)\s+(runs?|goals?|points?)\s+in\s+the\s+first\s+5\s+innings?$/i);
  if (m) {
    const ou = m[1][0].toUpperCase() + m[1].slice(1).toLowerCase();
    const linha = m[2].replace(",", ".");
    const unidade = unitFromWord(m[3]) || "runs";
    return `${matchup} - 1st 5 Innings ${ou} ${linha} ${unidade}`;
  }

  // "{Time} over/under N {unidade} scored" -> total do time (não do jogo).
  m = complemento.match(/^(.+?)\s+(over|under)\s+([0-9]+(?:[.,]\d+)?)\s+(runs?|goals?|points?)\s+scored$/i);
  if (m) {
    const team = findTeam(m[1]) || m[1].trim();
    const ou = m[2][0].toUpperCase() + m[2].slice(1).toLowerCase();
    const linha = m[3].replace(",", ".");
    const unidade = unitFromWord(m[4]) || "runs";
    return `${matchup} - ${team} ${ou} ${linha} ${unidade}`;
  }

  // Sem unidade explícita não sabemos se a linha é total, corrida, ponto ou
  // apenas um strike de mercado binário. Preservamos o título recebido para
  // nunca inventar "gols" em uma modalidade diferente.
  m = complemento.match(/^(over|under)\s+([0-9]+(?:[.,]\d+)?)$/i);
  if (m) {
    return raw;
  }

  // Complemento é só o nome de um dos dois times -> vencedor da partida (ML).
  const teamOnly = findTeam(complemento);
  const normalizedComplement = complemento.toLowerCase();
  const normalizedTeam = teamOnly.toLowerCase();
  if (teamOnly && (normalizedTeam === normalizedComplement || normalizedComplement.endsWith(" " + normalizedTeam))) {
    // A Kalshi pode abreviar o participante no confronto ("Alves") e
    // trazer o nome completo no mercado ("Mateus Alves"). Preservamos o
    // nome completo do contrato, desde que ele termine no participante do
    // confronto, para não confundir vencedor da partida com outro mercado.
    return `${matchup} - ${complemento} ML${outcomeRaw.toLowerCase() === 'no' ? ' No' : ''}`;
  }

  return raw;
}

// Cache de tituloExibicao: a normalização (Polymarket/Kalshi) roda várias
// dezenas de regex por posição. Sem cache, digitar na busca reprocessa TODAS
// as posições a cada tecla, mesmo que o título/outcome não tenham mudado
// desde o último cálculo — isso é o maior custo de CPU por interação no app.
// A chave inclui título+outcome+plataforma+id: qualquer mudança real nesses
// campos gera uma chave nova e recalcula do zero, então o resultado nunca
// fica desatualizado.
const _tituloExibicaoCache = new Map();
function tituloExibicao(d) {
if (!d) return "";
if (d._platform === "predict" && d._predictTitle) return d._predictTitle;
const drawTitle = globalThis.BettyMarketTitles?.drawTitle(d);
if (drawTitle) return drawTitle;
const extendedTitle = globalThis.BettyMarketTitles?.extendedTitle(d);
if (extendedTitle) return extendedTitle;
const specialTitle = globalThis.BettyMarketTitles?.kalshiSpecialTitle(d);
if (specialTitle) return specialTitle;
const inningTitle = globalThis.BettyMarketTitles?.inningWinnerTitle(d);
if (inningTitle) return inningTitle;
if (d._titleDisplay && !globalThis.BettyMarketTitles.kalshiTitleMissingMatchup(d,d._titleDisplay)) return d._titleDisplay;
const plat = String(d._platform || "polymarket").toLowerCase();
const idKey = d.conditionId || d.slug || d.ticker || d.market_ticker || "";
const meta = d._sportMeta || d._kalshiTitleMeta || null;
const cacheKey = plat + "\u0001" + (d.title || "") + "\u0001" + (d.outcome || "") + "\u0001" + idKey + "\u0001" + (d.asset || d.tokenId || '');
const cached = _tituloExibicaoCache.get(cacheKey);
if (cached && cached.meta === meta) return cached.title;
let resultado;
try {
if (plat === "polymarket") {
resultado = normalizarNomeMercadoPolymarket(d) || d.title || "";
} else if (plat === "kalshi") {
resultado = normalizarNomeMercadoKalshi(d) || d.title || "";
} else {
resultado = d.title || "";
}
} catch (erro) {
// Títulos são somente apresentação. Metadados inesperados de uma aposta não
// podem impedir a tabela nem a paginação de renderizar as demais posições.
console.warn("Título não normalizado:", erro);
resultado = d.title || "";
}
// Evita crescimento indefinido do cache em sessões muito longas.
if (_tituloExibicaoCache.size > 8000) _tituloExibicaoCache.clear();
_tituloExibicaoCache.set(cacheKey, { meta, title: resultado });
return resultado;
}
function getDadosAtuais() {
// Ativas e Fechadas são abas reais dentro de Posições: cada uma usa seu
// próprio snapshot (abertos/fechados).
const lista = abaAtual === "aberto" ? abertos : fechados;
return filtrarExcluidos(lista);
}
function filtrarPorPlataforma(lista) {
lista = filtrarExcluidos(lista);
const plat = filtroCasaAtual || "todas";
if (CASAS_STAKE.includes(plat)) return lista.filter(d => casaDaAposta(d) === plat);
return lista;
}
function atualizarFiltroEsportePosicoes() {
atualizarFiltrosCasasComApostas();
const select = document.getElementById("filtroEsporteSelect");
if (!select) return;
const selecionado = filtroEsportePosicoesAtual || "todos";
const categoriasPresentes = new Set();
// As abertas estão em memória; as fechadas permanentes vêm do resumo do
// SQLite, que já respeita o período configurado para cada casa.
for (const aposta of [...(abertos || []), ...(fechados || [])]) {
const categoria = classificarAposta(aposta);
if (categoria) categoriasPresentes.add(categoria);
}
for (const categoria of (resumoDashboardDb?.categorias || [])) {
if (categoria) categoriasPresentes.add(categoria);
}
const ordem = typeof ORDEM_CATEGORIA === "undefined" ? [] : ORDEM_CATEGORIA;
const opcoes = ordem.filter(categoria => categoriasPresentes.has(categoria))
.concat([...categoriasPresentes].filter(categoria => !ordem.includes(categoria)).sort((a, b) => a.localeCompare(b, "pt-BR")));
const assinatura = opcoes.join("\u001f");
if (select.dataset.categorias !== assinatura) {
select.innerHTML = `<option value="todos">Todos</option>` + opcoes.map(esporte =>
`<option value="${escapeHtml(esporte)}">${escapeHtml(esporte)}</option>`
).join("");
select.dataset.categorias = assinatura;
}
select.value = opcoes.includes(selecionado) ? selecionado : "todos";
filtroEsportePosicoesAtual = select.value;
}
function atualizarFiltrosCasasComApostas() {
// O resumo confirmado inclui o histórico completo, sem depender da página aberta.
if (!resumoDashboardDb) return;
const fechadas = new Set((resumoDashboardDb.porSite || [])
.filter(row => Number(row.total) > 0).map(row => row.chave));
const posicoes = new Set(fechadas);
for (const aposta of [...(abertos || []), ...(fechados || [])]) posicoes.add(aposta._platform || 'polymarket');
for (const [id, casas, valor] of [
['filtroCasaSelect', posicoes, filtroCasaAtual],
['profitCalendarHouse', fechadas, profitCalendarState.house]
]) {
const select = document.getElementById(id);
if (!select) continue;
const disponiveis = CASAS_STAKE.filter(casa => casas.has(casa));
const assinatura = disponiveis.join('|');
if (select.dataset.casasComApostas !== assinatura) {
const todas = id === 'profitCalendarHouse' ? 'Todas as casas' : 'Todas';
select.innerHTML = '<option value="todas">' + todas + '</option>' +
disponiveis.map(casa => '<option value="' + casa + '">' + escapeHtml(NOMES_CASAS[casa]) + '</option>').join('');
select.dataset.casasComApostas = assinatura;
}
select.value = casas.has(valor) ? valor : 'todas';
if (id === 'filtroCasaSelect') filtroCasaAtual = select.value;
else profitCalendarState.house = select.value;
}
}
function filtrarPorEsporte(lista) {
const esporte = filtroEsportePosicoesAtual || "todos";
if (esporte === "todos") return lista;
return lista.filter(d => classificarAposta(d) === esporte);
}
function alterarFiltroEsportePosicoes(valor) {
filtroEsportePosicoesAtual = valor || "todos";
paginaAtual = 1;
if (abaAtual === "ao-vivo") renderAoVivo();
else aplicarFiltros();
}
function toggleCasaPopup(ev) {
if (ev) ev.stopPropagation();
const popup = document.getElementById("casaPopup");
if (!popup) return;
// fecha o de data se estiver aberto
if (calendarioAberto) fecharPopupCalendario();
casaPopupAberto = !casaPopupAberto;
if (casaPopupAberto) {
popup.classList.add("open");
setTimeout(() => document.addEventListener("click", fecharCasaFora), 0);
} else {
fecharPopupCasa();
}
}
function fecharCasaFora(ev) {
const popup = document.getElementById("casaPopup");
const box = document.getElementById("filtroCasaBox");
if (popup && !popup.contains(ev.target) && box && !box.contains(ev.target)) {
fecharPopupCasa();
}
}
function fecharPopupCasa() {
const popup = document.getElementById("casaPopup");
if (popup) popup.classList.remove("open");
casaPopupAberto = false;
document.removeEventListener("click", fecharCasaFora);
}
function alterarFiltroCasa(valor) {
filtroCasaAtual = valor || "todas";
aplicarFiltros();
}
function selecionarCasa(valor, ev) {
if (ev) ev.stopPropagation();
filtroCasaAtual = valor;
const labels = { todas: "Todas", ...NOMES_CASAS };
const labelEl = document.getElementById("casaValueLabel");
if (labelEl) {
labelEl.textContent = labels[valor] || "Todas";
labelEl.classList.remove(...CASAS_STAKE.map(casa => "casa-" + casa));
if (CASAS_STAKE.includes(valor)) labelEl.classList.add("casa-" + valor);
}
const popup = document.getElementById("casaPopup");
if (popup) {
popup.querySelectorAll("button").forEach(b => {
b.classList.toggle("active", b.dataset.value === valor);
});
}
fecharPopupCasa();
aplicarFiltros();
}
function getPnl(d) {
const bruto = (abaAtual === "aberto" || abaAtual === "ao-vivo") ? d.cashPnl : d.realizedPnl;
const pnl = Number(bruto ?? 0);
return Number.isFinite(pnl) ? pnl : 0;
}
function getStake(d) {
// Stake em $ (não shares):
// Abertas: initialValue = $ investido
// Fechadas: totalBought é qtd de shares → multiplica por avgPrice
if (d.initialValue != null && d.initialValue > 0) return d.initialValue;
if (d.totalBought != null && d.avgPrice != null && d.avgPrice > 0) {
return d.totalBought * d.avgPrice;
}
if (d.size != null && d.avgPrice != null) return d.size * d.avgPrice;
return 0;
}
function getValorAtual(d) {
const cv = Number(d.currentValue);
if (Number.isFinite(cv) && cv >= 0) return cv;
const stake = getStake(d);
const pnl = Number(d.cashPnl);
return Math.max(0, stake + (Number.isFinite(pnl) ? pnl : 0));
}
function chaveMercado(d) {
const plat = (d._platform || "polymarket").toLowerCase();
return plat + "|" + String(d.eventSlug || d.slug || d.conditionId || tituloExibicao(d) || "").toLowerCase();
}
function getLucroSeVencer(d) {
if (d._platform === "sxbet") return d._potentialProfit ?? null;
// Em mercados binários, cada share paga US$ 1 se a seleção vencer.
// A estimativa é o retorno líquido: payout potencial menos o stake já pago.
const stake = getStake(d);
if (!Number.isFinite(stake) || stake <= 0) return null;
let shares = Number(d.size);
if (!Number.isFinite(shares) || shares <= 0) shares = Number(d.totalBought);
if ((!Number.isFinite(shares) || shares <= 0) && Number(d.avgPrice) > 0) shares = stake / Number(d.avgPrice);
if (!Number.isFinite(shares) || shares <= 0) return null;
return Math.abs(shares) - stake;
}
function getOdd(d) {
const p = d.avgPrice;
if (p == null || p <= 0) return "—";
// Odd decimal (estilo apostas BR)
const odd = 1 / p;
return odd.toFixed(2);
}
function atualizarSaldosFixos() {
for (const [id, valor] of [
["saldoPolyFixo", POLY_API_BALANCE],
["saldoKalshiFixo", KALSHI_API_BALANCE],
["saldoPredictFixo", PREDICT_API_BALANCE],
["saldoSxBetFixo", SXBET_API_BALANCE]
]) {
const el = document.getElementById(id);
if (!el) continue;
const confirmado = Number.isFinite(valor);
el.textContent = confirmado ? "$" + valor.toFixed(2) : "—";
el.classList.toggle("positive", confirmado && valor >= 0);
el.classList.toggle("negative", confirmado && valor < 0);
}
const predictCard = document.getElementById("saldoPredictCard");
if (predictCard) {
predictCard.hidden = !BETTY_CONNECTIONS?.predict;
}
const sxBetCard = document.getElementById("saldoSxBetCard");
if (sxBetCard) sxBetCard.hidden = !BETTY_CONNECTIONS?.sxbet;
}
const ICONS = {
trendUp: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 17l6-6 4 4 8-8"/><path d="M17 7h4v4"/></svg>',
trendDown: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 7l6 6 4-4 8 8"/><path d="M17 17h4v-4"/></svg>',
db: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><ellipse cx="12" cy="5" rx="8" ry="3"/><path d="M4 5v14c0 1.66 3.58 3 8 3s8-1.34 8-3V5"/><path d="M4 12c0 1.66 3.58 3 8 3s8-1.34 8-3"/></svg>',
wallet: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 7a2 2 0 0 1 2-2h13a1 1 0 0 1 1 1v3"/><path d="M3 7v10a2 2 0 0 0 2 2h14a1 1 0 0 0 1-1v-4a1 1 0 0 0-1-1h-4a2 2 0 1 1 0-4h4a1 1 0 0 0 1-1"/></svg>',
percent: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M5 19L19 5"/><circle cx="7" cy="7" r="2"/><circle cx="17" cy="17" r="2"/></svg>',
target: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="9"/><circle cx="12" cy="12" r="5"/><circle cx="12" cy="12" r="1" fill="currentColor"/></svg>'
};
let resumoDashboardDb = null;
let resumoDashboardDbChave = "";
let resumoDashboardDbEmAndamento = null;
let resumoPeriodoDash = null;
let resumoPeriodoDashEmAndamento = null;
function ymdOffset(daysFromToday) {
const hoje = ymdBrasil(new Date());
const [y, m, dd] = hoje.split("-").map(Number);
const dt = new Date(Date.UTC(y, m - 1, dd + Number(daysFromToday || 0)));
return `${dt.getUTCFullYear()}-${String(dt.getUTCMonth() + 1).padStart(2, "0")}-${String(dt.getUTCDate()).padStart(2, "0")}`;
}
async function carregarResumoDashboardDb() {
if (!indiceHistoricoLegivel()) return null;
const chave = chavePeriodoFechadasConfigurado();
if (resumoDashboardDb && resumoDashboardDbChave === chave) return resumoDashboardDb;
const versao = versaoLeituraHistorico();
if (resumoDashboardDbEmAndamento?.chave === chave && resumoDashboardDbEmAndamento.versao === versao) return resumoDashboardDbEmAndamento.promise;
const promise = fetch(urlLeituraHistorico('/api/history/closed/stats'), { cache: 'no-store' })
.then(r => { if (!r.ok) throw new Error('resumo HTTP ' + r.status); return r.json(); })
.then(data => {
if (!data?.ok) throw new Error('resumo inválido');
if (!resumoHistoricoPodeSerPublicado(versao)) return null;
if (chavePeriodoFechadasConfigurado() === chave) {
resumoDashboardDb = data;
resumoDashboardDbChave = chave;
atualizarFiltroEsportePosicoes();
}
return data;
})
.catch(e => { console.warn('Resumo do histórico no SQLite indisponível:', e); return null; })
.finally(() => { if (resumoDashboardDbEmAndamento?.chave === chave && resumoDashboardDbEmAndamento?.versao === versao) resumoDashboardDbEmAndamento = null; });
resumoDashboardDbEmAndamento = { chave, versao, promise };
return promise;
}
async function carregarResumoPeriodoDash() {
if (!indiceHistoricoLegivel()) return null;
const start = ymdOffset(-6), end = ymdBrasil(new Date());
const chave = [start, end, chavePeriodoFechadasConfigurado()].join("|");
if (resumoPeriodoDash?.chave === chave) return resumoPeriodoDash.data;
const versao = versaoLeituraHistorico();
if (resumoPeriodoDashEmAndamento?.chave === chave && resumoPeriodoDashEmAndamento.versao === versao) return resumoPeriodoDashEmAndamento.promise;
const params = new URLSearchParams({ start, end, includePrevious: "1" });
const promise = fetch(urlLeituraHistorico('/api/history/closed/stats?' + params.toString()), { cache: 'no-store' })
.then(r => { if (!r.ok) throw new Error('período HTTP ' + r.status); return r.json(); })
.then(data => {
if (!data?.ok) throw new Error('período inválido');
if (!resumoHistoricoPodeSerPublicado(versao)) return null;
resumoPeriodoDash = { chave, data };
return data;
}).catch(e => { console.warn('PnL do período indisponível:', e); return null; })
.finally(() => { if (resumoPeriodoDashEmAndamento?.chave === chave && resumoPeriodoDashEmAndamento?.versao === versao) resumoPeriodoDashEmAndamento = null; });
resumoPeriodoDashEmAndamento = { chave, versao, promise };
return promise;
}
function fmtSinalDinheiro(n) {
const v = Number(n) || 0;
return (v >= 0 ? "+$" : "-$") + Math.abs(v).toFixed(2);
}
function renderizarDashboardSincronizandoHistorico() {
const statsEl = document.getElementById("stats");
// Um painel já preenchido representa a última leitura confirmada e deve
// permanecer visível. Já um esqueleto de carregamento ainda não representa
// números financeiros, então o substituímos pelo estado de sincronização.
if (!statsEl || (statsEl.innerHTML.trim() && !statsEl.querySelector(".kpi-vazio"))) return;
statsEl.innerHTML = kpisVazios("Sincronizando o histórico…");
}
async function renderizarStats(opcoes = {}) {
const pintar = opcoes.pintar ?? dashboardEstaVisivel();
if (!pintar) dashboardVisualPendente = true;
// Um refresh pode reescrever o índice em diversos lotes. Também não aceitamos
// o cache inicial do navegador como fonte de números financeiros. Mantemos os
// cards já confirmados e, na primeira abertura, mostramos apenas carregamento
// até que todas as métricas venham da mesma versão do histórico no SQLite.
if (!dadosFinanceirosConfirmados()) {
if (pintar) renderizarDashboardSincronizandoHistorico();
return;
}
const versaoHistorico = versaoLeituraHistorico();
if (pintar) atualizarSaldosFixos();
const abertosF = filtrarExcluidos(Array.isArray(abertos) ? abertos : []);
const [resumoDb, periodoDb] = await Promise.all([carregarResumoDashboardDb(), carregarResumoPeriodoDash()]);
if (!leituraHistoricoAindaAtual(versaoHistorico)) return;
// As consultas acima podem preparar o Dashboard. Somatórios, títulos e HTML
// dos cards só são necessários quando essa tela está visível.
if (!pintar || !dashboardEstaVisivel()) { dashboardVisualPendente = true; return; }
const fechadoDb = resumoDb?.total || null;
const fechadosF = fechadoDb ? [] : apostasFechadasNoSite();
if (!snapshotAoVivoConfirmado && !abertosF.length && !fechadoDb && !fechadosF.length) {
const statsEl = document.getElementById("stats");
if (pintar && statsEl && !statsEl.innerHTML.trim()) {
statsEl.innerHTML = kpisVazios("Carregando…");
}
return;
}
const nAbertas = abertosF.length;
const casasAtivas = casasConectadas();
const casasAbertas = Object.fromEntries(casasAtivas.map(casa => [casa, { count: 0, stake: 0 }]));
for (const d of abertosF) {
const casa = casaDaAposta(d);
const grupo = casasAbertas[casa] ||= { count: 0, stake: 0 };
grupo.count += 1;
grupo.stake += getStake(d);
}
const nFechadas = fechadoDb ? (Number(fechadoDb.total) || 0) : fechadosF.length;
const stakeAbertas = abertosF.reduce((s, d) => s + getStake(d), 0);
const saldos = { polymarket: POLY_API_BALANCE, kalshi: KALSHI_API_BALANCE, predict: PREDICT_API_BALANCE };
const casasPatrimonio = casasAtivas.filter(casa => Number.isFinite(saldos[casa]));
const patrimonioParcial = casasPatrimonio.length !== casasAtivas.length;
const valorAberto = abertosF.filter(d => casasPatrimonio.includes(casaDaAposta(d))).reduce((s, d) => s + getValorAtual(d), 0);
const pnlNaoRealizado = abertosF.reduce((s, d) => s + (Number(d.cashPnl) || 0), 0);
const pnlRealizado = fechadoDb ? (Number(fechadoDb.lucroD) || 0) : fechadosF.reduce((s, d) => s + (d.realizedPnl || 0), 0);
const stakeFechadas = fechadoDb ? (Number(fechadoDb.volumeD) || 0) : fechadosF.reduce((s, d) => s + getStake(d), 0);
const saldoPatrimonio = casasPatrimonio.reduce((total, casa) => total + saldos[casa], 0);
const patrimonio = saldoPatrimonio + valorAberto;
const roiGeral = stakeFechadas > 0 ? ((pnlRealizado / stakeFechadas) * 100) : 0;
const winsGeral = fechadoDb ? (Number(fechadoDb.wins) || 0) : fechadosF.filter(d => (d.realizedPnl || 0) > 0).length;
const winRateGeral = fechadoDb ? (Number(fechadoDb.taxaAcerto) || 0) : (nFechadas ? ((winsGeral / nFechadas) * 100) : 0);
const pnlHoje = fechadoDb ? (Number(fechadoDb.pnlHoje) || 0) : 0;
const pnlPeriodo = periodoDb?.total ? Number(periodoDb.total.lucroD) || 0 : 0;
const pnlAnterior = periodoDb?.anterior ? Number(periodoDb.anterior.lucroD) || 0 : null;
const porMercado = {};
for (const d of abertosF) {
const k = chaveMercado(d);
if (!porMercado[k]) porMercado[k] = { stake: 0, titulo: tituloExibicao(d) || "—" };
porMercado[k].stake += getStake(d);
}
const concMercado = Object.values(porMercado).sort((a, b) => b.stake - a.stake)[0] || null;
const drawdownAtual = Number(resumoDb?.avancadas?.currentDrawdown);
if (pintar) pintarVisaoGeral({
abertosF, casasAtivas, casasAbertas, stakeAbertas, patrimonio, patrimonioParcial, casasPatrimonio,
saldoPatrimonio, valorAberto, pnlNaoRealizado, pnlRealizado, roiGeral, pnlHoje, pnlPeriodo, pnlAnterior,
periodoDb, nFechadas, drawdownAtual, maxDrawdown: Number(resumoDb?.avancadas?.maxDrawdown),
concMercado: concMercado && stakeAbertas > 0 ? concMercado : null
});
const oddsValidas = fechadoDb ? [] : fechadosF.map(d => getOddNum(d)).filter(o => o > 0);
const oddMedia = fechadoDb ? (Number(fechadoDb.oddMedia) || 0) : (oddsValidas.length ? (oddsValidas.reduce((s, o) => s + o, 0) / oddsValidas.length) : 0);
const stakeMedia = nFechadas > 0 ? (stakeFechadas / nFechadas) : 0;
const taxaEl = document.getElementById("resumoTaxa");
if (pintar && taxaEl) taxaEl.textContent = winRateGeral.toFixed(0) + "%";
const oddEl = document.getElementById("resumoOdd");
if (pintar && oddEl) oddEl.textContent = oddMedia > 0 ? oddMedia.toFixed(2) : "—";
const stakeMediaEl = document.getElementById("resumoStakeMedia");
if (pintar && stakeMediaEl) stakeMediaEl.textContent = stakeMedia > 0 ? '$' + stakeMedia.toFixed(2) : "—";
if (pintar) dashboardVisualPendente = false;
}
function ymdBrasil(dt) {
if (!dt) return null;
ymdBrasil.formato ||= new Intl.DateTimeFormat("en-CA", {
timeZone: "America/Sao_Paulo",
year: "numeric",
month: "2-digit",
day: "2-digit",
});
return ymdBrasil.formato.format(dt); // YYYY-MM-DD
}
function apostaNoDia(d, ymd) {
if (!ymd) return true;
const dt = dataPosicaoTabela(d);
if (!dt) return false;
return ymdBrasil(dt) === ymd;
}
function renderFiltroDataBox() {
const valEl = document.getElementById("dataValueLabel");
if (!valEl) return;
const box = document.getElementById("filtroDataBox");
if (filtroDataDia) {
const [yy, mm, dd] = filtroDataDia.split("-");
valEl.textContent = `${dd}/${mm}/${yy}`;
valEl.classList.add("active");
if (box) box.setAttribute("aria-label", `Filtrar por data: ${dd}/${mm}/${yy}`);
} else {
valEl.textContent = "Todas";
valEl.classList.remove("active");
if (box) box.setAttribute("aria-label", "Filtrar por data: Todas");
}
}
function toggleDatePopup(ev) {
if (ev) ev.stopPropagation();
fecharExportPopup();
const popup = document.getElementById("datePopup");
if (!popup) return;
if (casaPopupAberto) fecharPopupCasa();
calendarioAberto = !calendarioAberto;
if (calendarioAberto) {
if (filtroDataDia) {
const [yy, mm] = filtroDataDia.split("-");
calendarioMes = new Date(parseInt(yy, 10), parseInt(mm, 10) - 1, 1);
} else {
const hoje = new Date();
calendarioMes = new Date(hoje.getFullYear(), hoje.getMonth(), 1);
}
renderCalendarPopup();
popup.classList.add("open");
const box = document.getElementById("filtroDataBox");
if (box) box.setAttribute("aria-expanded", "true");
requestAnimationFrame(() => posicionarDatePopup());
document.addEventListener("scroll", fecharPopupCalendario, true);
setTimeout(() => document.addEventListener("click", fecharCalendarioFora), 0);
} else {
fecharPopupCalendario();
}
}
function fecharCalendarioFora(ev) {
const popup = document.getElementById("datePopup");
const box = document.getElementById("filtroDataBox");
if (popup && !popup.contains(ev.target) && box && !box.contains(ev.target)) {
fecharPopupCalendario();
}
}
function fecharPopupCalendario() {
const popup = document.getElementById("datePopup");
if (popup) {
popup.classList.remove("open");
popup.style.top = "";
popup.style.left = "";
}
const box = document.getElementById("filtroDataBox");
if (box) box.setAttribute("aria-expanded", "false");
calendarioAberto = false;
document.removeEventListener("click", fecharCalendarioFora);
document.removeEventListener("scroll", fecharPopupCalendario, true);
}
function navegarMesCalendario(delta, ev) {
if (ev) ev.stopPropagation();
calendarioMes = new Date(calendarioMes.getFullYear(), calendarioMes.getMonth() + delta, 1);
renderCalendarPopup();
}
function posicionarDatePopup() {
const popup = document.getElementById("datePopup");
const box = document.getElementById("filtroDataBox");
if (!popup || !box || !popup.classList.contains("open")) return;
const r = box.getBoundingClientRect();
const gap = 8;
const margin = 8;
const w = popup.offsetWidth || 260;
const h = popup.offsetHeight || 330;
let left = r.left;
let top = r.bottom + gap;
if (left + w > window.innerWidth - margin) left = window.innerWidth - w - margin;
if (left < margin) left = margin;
if (top + h > window.innerHeight - margin) top = Math.max(margin, r.top - h - gap);
popup.style.left = `${left}px`;
popup.style.top = `${top}px`;
}
function renderCalendarPopup() {
const popup = document.getElementById("datePopup");
if (!popup) return;
const ano = calendarioMes.getFullYear();
const mes = calendarioMes.getMonth();
const nomesMes = ["janeiro", "fevereiro", "março", "abril", "maio", "junho", "julho", "agosto", "setembro", "outubro", "novembro", "dezembro"];
const primeiroDiaSemana = new Date(ano, mes, 1).getDay();
const diasNoMes = new Date(ano, mes + 1, 0).getDate();
const diasMesAnterior = new Date(ano, mes, 0).getDate();
const hojeYmd = ymdBrasil(new Date());
const pad2 = n => String(n).padStart(2, "0");
let cells = "";
for (let i = primeiroDiaSemana - 1; i >= 0; i--) {
cells += `<div class="day other">${diasMesAnterior - i}</div>`;
}
for (let d = 1; d <= diasNoMes; d++) {
const ymd = `${ano}-${pad2(mes + 1)}-${pad2(d)}`;
let cls = "day";
if (ymd === hojeYmd) cls += " today";
if (filtroDataDia === ymd) cls += " selected";
cells += `<div class="${cls}" data-on-click="selecionarDiaCalendario('${ymd}', event)">${d}</div>`;
}
const resto = (primeiroDiaSemana + diasNoMes) % 7;
if (resto !== 0) {
for (let d = 1; d <= 7 - resto; d++) cells += `<div class="day other">${d}</div>`;
}
popup.innerHTML = `
<div class="date-popup-head">
<span>${nomesMes[mes]} de ${ano}</span>
<div class="date-popup-nav">
<button type="button" data-on-click="navegarMesCalendario(-1, event)">‹</button>
<button type="button" data-on-click="navegarMesCalendario(1, event)">›</button>
</div>
</div>
<div class="date-grid">
<div class="dow">D</div><div class="dow">S</div><div class="dow">T</div><div class="dow">Q</div><div class="dow">Q</div><div class="dow">S</div><div class="dow">S</div>
${cells}
</div>
<div class="date-popup-foot">
<button type="button" class="limpar" data-on-click="limparFiltroData(event)">Limpar</button>
<button type="button" data-on-click="selecionarDiaCalendario('${hojeYmd}', event)">Hoje</button>
</div>
`;
}
function selecionarDiaCalendario(ymd, ev) {
if (ev) ev.stopPropagation();
filtroDataDia = (filtroDataDia === ymd) ? null : ymd;
renderFiltroDataBox();
fecharPopupCalendario();
if (abaAtual === "ao-vivo") renderAoVivo();
else aplicarFiltros();
}
function limparFiltroData(ev) {
if (ev) ev.stopPropagation();
filtroDataDia = null;
renderFiltroDataBox();
fecharPopupCalendario();
if (abaAtual === "ao-vivo") renderAoVivo();
else aplicarFiltros();
}
// manterPagina=true: refresh automático não pode tirar o usuário da página
// em que ele está. Só uma mudança de filtro/ordem volta para a página 1.
function ordenacaoHistoricoFechadoDb() {
if (sortCol) return { sort: sortCol, dir: sortDir };
const ordem = document.getElementById("ordenar")?.value || "inicio-asc";
if (ordem === "pnl-desc") return { sort: "pnl", dir: "desc" };
if (ordem === "pnl-asc") return { sort: "pnl", dir: "asc" };
if (ordem === "stake-desc") return { sort: "stake", dir: "desc" };
if (ordem === "titulo-asc") return { sort: "titulo", dir: "asc" };
return { sort: "inicio-asc", dir: "desc" };
}
const paginasHistoricoPosicoes = new Map();
let escopoPaginasHistoricoPosicoes = '';
let preparoPaginaFechadasTimer = null;
function escopoConsultaPosicoes() {
return JSON.stringify([BETTY_USER?.id || 'local',exibindoDadosSalvos ? 'salvos' : refreshGeneration,versaoLeituraHistorico(),chavePeriodoFechadasConfigurado()]);
}
function agendarPreparoNavegacao() {
const previous=agendarPreparoNavegacao.handle;
if(previous){if(previous.idle)window.cancelIdleCallback(previous.id);else clearTimeout(previous.id);}
agendarPreparoNavegacao.handle=null;
if(!dadosFinanceirosConfirmados() || document.hidden)return;
const scope=escopoConsultaPosicoes();
const run=async()=>{
agendarPreparoNavegacao.handle=null;
if(!dadosFinanceirosConfirmados() || document.hidden || scope!==escopoConsultaPosicoes())return;
try {
await carregarResumoEstatDb({incluirMercados:true});
if(scope===escopoConsultaPosicoes() && dadosFinanceirosConfirmados())await prepararProfitCalendar();
}catch(_){/* Optional preparation never publishes failures or blocks navigation. */}
};
if(window.requestIdleCallback)agendarPreparoNavegacao.handle={idle:true,id:window.requestIdleCallback(run,{timeout:1500})};
else agendarPreparoNavegacao.handle={idle:false,id:setTimeout(run,200)};
}
function parametrosConsultaFechadas(pagina = paginaAtual, ordem = ordenacaoHistoricoFechadoDb()) {
const termo = document.getElementById("filtroTitulo")?.value.toLowerCase().trim() || "";
const params = new URLSearchParams({
platform: filtroCasaAtual || "todas", category: filtroEsportePosicoesAtual || "todos", title: termo, day: filtroDataDia || "",
sort: ordem.sort, dir: ordem.dir, page: String(pagina), pageSize: String(itensPorPagina())
});
return params;
}
function consultarPaginaFechadas(params) {
const escopo = escopoConsultaPosicoes();
if (escopoPaginasHistoricoPosicoes !== escopo) {
paginasHistoricoPosicoes.clear();
escopoPaginasHistoricoPosicoes = escopo;
}
const chaveParams = new URLSearchParams(params);
chaveParams.delete('layout');
const chave = chaveParams.toString();
const existente = paginasHistoricoPosicoes.get(chave);
if (existente) {
paginasHistoricoPosicoes.delete(chave);
paginasHistoricoPosicoes.set(chave,existente);
return existente.promise;
}
const entrada = {bytes:0};
entrada.promise = (async()=>{
const res = await fetch(urlLeituraHistorico('/api/history/closed/query?' + params.toString()), { cache: 'no-store' });
if (!res.ok) throw new Error('histórico HTTP ' + res.status);
const data = await res.json();
if (!data?.ok || !Array.isArray(data.fechados)) throw new Error('histórico inválido');
entrada.bytes = Number(res.headers.get('content-length')) || 0;
if (escopoConsultaPosicoes() === escopo && paginasHistoricoPosicoes.get(chave) === entrada) {
// Limite de memória: no máximo três páginas e 8 MiB de payload recebido.
if (!entrada.bytes || entrada.bytes > 8 * 1024 * 1024) paginasHistoricoPosicoes.delete(chave);
let bytes = [...paginasHistoricoPosicoes.values()].reduce((n,item)=>n+item.bytes,0);
while (paginasHistoricoPosicoes.size > 3 || bytes > 8 * 1024 * 1024) {
const primeira = paginasHistoricoPosicoes.keys().next().value;
bytes -= paginasHistoricoPosicoes.get(primeira).bytes;
paginasHistoricoPosicoes.delete(primeira);
}
}
return data;
})().catch(error=>{
if (paginasHistoricoPosicoes.get(chave) === entrada) paginasHistoricoPosicoes.delete(chave);
throw error;
});
paginasHistoricoPosicoes.set(chave,entrada);
return entrada.promise;
}
function prepararPrimeiraPaginaFechadas() {
clearTimeout(preparoPaginaFechadasTimer);
if (viewAtual !== 'posicoes' || abaAtual !== 'aberto' || !dadosFinanceirosConfirmados()) return;
// Deixa Ativas pintar antes da preparação. Não publica números nem altera
// a aba, paginação ou ordenação atuais. Fechadas sempre abre na página 1.
preparoPaginaFechadasTimer = setTimeout(()=>{
if (viewAtual !== 'posicoes' || abaAtual !== 'aberto' || !dadosFinanceirosConfirmados()) return;
consultarPaginaFechadas(parametrosConsultaFechadas(1,{sort:'inicio-asc',dir:'desc'})).catch(()=>{});
},0);
}
let preparoPaginasVizinhasTimer = null;
function prepararPaginasVizinhasFechadas(total) {
clearTimeout(preparoPaginasVizinhasTimer);
const params = parametrosConsultaFechadas();
const escopo = escopoConsultaPosicoes();
const pagina = Number(params.get('page'));
const totalPaginas = Math.ceil(total / Number(params.get('pageSize')));
preparoPaginasVizinhasTimer = setTimeout(()=>{
if (abaAtual !== 'fechado' || viewAtual !== 'posicoes' || !dadosFinanceirosConfirmados() ||
escopoConsultaPosicoes() !== escopo || parametrosConsultaFechadas().toString() !== params.toString()) return;
for (const proxima of [pagina+1,pagina-1]) {
if (proxima < 1 || proxima > totalPaginas) continue;
const vizinha = new URLSearchParams(params);
vizinha.set('page',String(proxima));
consultarPaginaFechadas(vizinha).catch(()=>{});
}
},80);
}
async function aplicarFiltrosHistoricoFechadoDb(opcoes = {}) {
if (historicoIndiceEmSincronizacao && !exibindoDadosSalvos) return;
const geracao = ++consultaHistoricoFechadoGeracao;
const versao = versaoLeituraHistorico();
const escopo = escopoConsultaPosicoes();
try {
const data = await consultarPaginaFechadas(parametrosConsultaFechadas());
if (geracao !== consultaHistoricoFechadoGeracao || abaAtual !== "fechado" || escopoConsultaPosicoes() !== escopo || !resumoHistoricoPodeSerPublicado(versao)) return;
const total = Number(data.total) || 0;
const porPagina = itensPorPagina();
const totalPaginas = Math.max(1, Math.ceil(total / porPagina));
if (paginaAtual > totalPaginas) { paginaAtual = totalPaginas; return aplicarFiltrosHistoricoFechadoDb({ manterPagina: true }); }
listaFiltradaAtual = data.fechados;
renderizarStats();
renderizarTabela(data.fechados);
atualizarRodapePaginacao(total, (paginaAtual - 1) * porPagina, data.fechados.length, totalPaginas);
prepararPaginasVizinhasFechadas(total);
} catch (e) {
if (geracao !== consultaHistoricoFechadoGeracao || abaAtual !== "fechado" || escopoConsultaPosicoes() !== escopo || !resumoHistoricoPodeSerPublicado(versao)) return;
// Enquanto o índice é criado ou se a consulta local falhar, preservamos o
// comportamento anterior e nunca mostramos uma tabela vazia por engano.
console.warn('Consulta paginada do histórico indisponível:', e);
// Os dados salvos continuam valendo; a tabela fica como está.
if (exibindoDadosSalvos) return;
historicoFechadoIndexado = false;
aplicarFiltrosFrontend(opcoes);
}
}
function aplicarFiltrosFrontend(opcoes = {}) {
let lista = [...getDadosAtuais()];
if (abaAtual === "fechado") lista = lista.filter(apostaNoPeriodoFechadasConfigurado);
const termo = document.getElementById("filtroTitulo").value.toLowerCase().trim();
if (termo) {
lista = lista.filter(d => tituloExibicao(d).toLowerCase().includes(termo));
}
lista = filtrarPorPlataforma(lista);
lista = filtrarPorEsporte(lista);
if (filtroDataDia) {
lista = lista.filter(d => apostaNoDia(d, filtroDataDia));
}
lista.sort((a, b) => compararLinhas(a, b));
agruparAbertasPorCasa(lista);
listaFiltradaAtual = lista;
if (!opcoes.manterPagina) paginaAtual = 1;
renderizarStats();
renderizarPagina();
prepararPrimeiraPaginaFechadas();
}
function aplicarFiltros(opcoes = {}) {
if (abaAtual === "ao-vivo") return renderAoVivo(opcoes);
if (abaAtual === "fechado" && historicoIndiceEmSincronizacao && !exibindoDadosSalvos) return;
if (abaAtual === "fechado" && indiceDisponivel()) return aplicarFiltrosHistoricoFechadoDb(opcoes);
return aplicarFiltrosFrontend(opcoes);
}
// Digitar na busca chama aplicarFiltros() a cada tecla; com muitas posições
// isso refiltra+reordena+re-renderiza a lista inteira em cada uma. O debounce
// espera uma pequena pausa na digitação antes de rodar, então uma sequência
// rápida de teclas dispara só 1 execução em vez de N — sem mudar o resultado
// final, só quando ele é calculado.
let _aplicarFiltrosDebounceTimer = null;
function aplicarFiltrosDebounced() {
if (_aplicarFiltrosDebounceTimer) clearTimeout(_aplicarFiltrosDebounceTimer);
_aplicarFiltrosDebounceTimer = setTimeout(() => {
_aplicarFiltrosDebounceTimer = null;
aplicarFiltros();
}, 160);
}
// Kalshi/Poly às vezes mandam ISO sem "Z" (UTC). Sem isso o browser
// interpreta como horário local e aparece +3h no Brasil.
function parseApiDate(raw) {
if (!raw) return null;
let s = String(raw).trim();
if (/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}/.test(s) && !/[zZ]|[+-]\d{2}:?\d{2}$/.test(s)) {
s += "Z";
}
const dt = new Date(s);
return isNaN(dt.getTime()) ? null : dt;
}
// Séries EUA: horário no ticker é ET
function isKalshiUSSeries(ticker) {
const t = String(ticker || "").toUpperCase();
return /KXMLB|KXNBA|KXNFL|KXNHL|KXNWSL|KXMLS|KXWNBA|KXNCAAF|KXNCAAB/.test(t);
}
// Ex.: KXMLBTEAMTOTAL-26AUG081805TORPHI → 08/08 18:05 ET → 19:05 BRT
// Só séries americanas com HHMM no ticker
function startFromKalshiTicker(ticker) {
if (!ticker || !isKalshiUSSeries(ticker)) return null;
const m = String(ticker).match(
/(\d{2})(JAN|FEB|MAR|APR|MAY|JUN|JUL|AUG|SEP|OCT|NOV|DEC)(\d{2})(\d{2})(\d{2})/i
);
if (!m) return null;
const year = 2000 + parseInt(m[1], 10);
const monMap = { JAN:0,FEB:1,MAR:2,APR:3,MAY:4,JUN:5,JUL:6,AUG:7,SEP:8,OCT:9,NOV:10,DEC:11 };
const mon = monMap[m[2].toUpperCase()];
if (mon == null) return null;
const day = parseInt(m[3], 10);
const hour = parseInt(m[4], 10);
const min = parseInt(m[5], 10);
if (hour > 23 || min > 59) return null;
const pad = n => String(n).padStart(2, "0");
const offset = (mon >= 2 && mon <= 10) ? "-04:00" : "-05:00";
const iso = `${year}-${pad(mon + 1)}-${pad(day)}T${pad(hour)}:${pad(min)}:00${offset}`;
const dt = new Date(iso);
return isNaN(dt.getTime()) ? null : dt;
}
// Tokens de time no título (ex.: "Lazio vs Ostia: Total Goals")
function tokensJogo(title) {
if (!title) return [];
return String(title)
.toLowerCase()
.replace(/[^a-z0-9\s]/g, " ")
.split(/\s+/)
.filter(w => w.length >= 3)
.filter(w => !/^(the|total|goals|runs|over|under|spread|wins?|will|from|with|team|match|regulation|time|scored|points|moneyline)$/.test(w));
}
// Horário da Kalshi: usa exclusivamente a data/hora oficial fornecida pela Kalshi.
// Não cruza com a Polymarket. O milestone.start_date é RFC3339/UTC e o navegador
// converte esse instante para America/Sao_Paulo na exibição.
function dataInicioAposta(d) {
const plat = (d._platform || "polymarket").toLowerCase();
if (plat === "kalshi") {
const raw = parseApiDate(d._startDate || null);
if (raw) return raw;
// Fallback SOMENTE para linhas que ainda ficaram sem data.
// Não altera nenhum horário que já veio da Kalshi. Algumas séries
// americanas da Kalshi codificam data/hora do jogo no ticker.
const tickerFallback = startFromKalshiTicker(d.ticker || d.slug || d.eventSlug || "");
if (tickerFallback) return tickerFallback;
return null;
}
return parseApiDate(d._startDate || d.endDate || null);
}
function getDataTs(d) {
const dt = dataInicioAposta(d);
if (dt) return dt.getTime();
// Fallback: timestamp de resgate/trade
if (d.timestamp) return d.timestamp * 1000;
if (d._lastTradeTs) return d._lastTradeTs * (d._lastTradeTs < 1e12 ? 1000 : 1);
return 0;
}
function dataPosicaoTabela(d, aberta = abaAtual !== "fechado") {
if (aberta) return dataInicioAposta(d);
// Mesma data de realização dos gráficos. Sem data confiável, não usa o jogo.
const ts = d?._positionClosedTs != null ? Number(d._positionClosedTs) : getTsResultadoPnL(d);
return Number.isFinite(ts) && ts > 0 ? new Date(ts) : null;
}
function getDataTsPosicao(d) {
if (abaAtual !== "fechado") return getDataTs(d);
return dataPosicaoTabela(d, false)?.getTime() || 0;
}
function getOddNum(d) {
const p = d.avgPrice;
if (p == null || p <= 0) return 0;
return 1 / p;
}
function compararLinhas(a, b) {
// Clique no cabeçalho tem prioridade
if (sortCol) {
let cmp = 0;
if (sortCol === "data") cmp = getDataTsPosicao(a) - getDataTsPosicao(b);
else if (sortCol === "titulo") cmp = (a.title || "").localeCompare(b.title || "", "pt-BR");
else if (sortCol === "odd") cmp = getOddNum(a) - getOddNum(b);
else if (sortCol === "stake") cmp = getStake(a) - getStake(b);
else if (sortCol === "pnl") cmp = getPnl(a) - getPnl(b);
return sortDir === "asc" ? cmp : -cmp;
}
const ordem = document.getElementById("ordenar").value;

if (ordem === "inicio-asc") {
// Ativas: evento mais cedo primeiro. Fechadas: realização mais recente.
const ta = getDataTsPosicao(a) || 0;
const tb = getDataTsPosicao(b) || 0;
if (abaAtual === "fechado") return tb - ta;
return (ta || Infinity) - (tb || Infinity);
}
if (ordem === "pnl-desc") return getPnl(b) - getPnl(a);
if (ordem === "pnl-asc") return getPnl(a) - getPnl(b);
if (ordem === "stake-desc") return getStake(b) - getStake(a);
if (ordem === "titulo-asc") return (a.title || "").localeCompare(b.title || "");
return 0;
}
function ordenarPorColuna(col) {
if (sortCol === col) {
sortDir = sortDir === "asc" ? "desc" : "asc";
} else {
sortCol = col;
// Texto/data: A→Z / mais cedo; números: maior primeiro
sortDir = (col === "titulo" || col === "data") ? "asc" : "desc";
}
aplicarFiltros();
}
function montarThData() {
const sortActive = sortCol === "data";
const sortInd = sortActive ? (sortDir === "asc" ? "▲" : "▼") : "⇅";
const sortCls = sortActive ? " sorted" : "";
const tituloData = abaAtual === "fechado" ? "Data de fechamento" : "Data do evento";
const ajudaData = abaAtual === "fechado" ? ' title="Data da venda ou resolução da aposta"' : ' title="Data e horário da partida ou evento"';
return `<th class="col-data sortable${sortCls}"${ajudaData} data-on-click="ordenarPorColuna('data')">${tituloData}<span class="sort-ind">${sortInd}</span></th>`;
}
function montarThead() {
function th(col, label, extraClass) {
const active = sortCol === col;
const ind = active ? (sortDir === "asc" ? "▲" : "▼") : "⇅";
const sorted = active ? " sorted" : "";
return `<th class="${extraClass} sortable${sorted}" data-on-click="ordenarPorColuna('${col}')">${label}<span class="sort-ind">${ind}</span></th>`;
}
return `<thead><tr>
${montarThData()}
<th class="col-casa">Casa</th>
${th("titulo", "Jogo", "col-jogo")}
<th class="col-aposta">Aposta</th>
${th("odd", "Odd", "col-mid")}
${th("stake", "Stake ($)", "col-mid col-stake")}
${abaAtual === "ao-vivo" ? '<th class="col-profit" title="Lucro líquido estimado se a aposta vencer">Profit</th>' : ""}
${th("pnl", "PnL", "col-num")}
</tr></thead>`;
}
function quantidadeColunasTabela() {
return abaAtual === "ao-vivo" ? 8 : 7;
}
function classeTabelaAtual() {
return abaAtual === "ao-vivo" ? ' class="positions-table live-positions-table"' : ' class="positions-table"';
}
function renderizarPagina() {
const porPagina = itensPorPagina();
const total = listaFiltradaAtual.length;
const totalPaginas = Math.max(1, Math.ceil(total / porPagina));
if (paginaAtual > totalPaginas) paginaAtual = totalPaginas;
if (paginaAtual < 1) paginaAtual = 1;
const startIdx = (paginaAtual - 1) * porPagina;
const slice = listaFiltradaAtual.slice(startIdx, startIdx + porPagina);
renderizarTabela(slice);
atualizarRodapePaginacao(total, startIdx, slice.length, totalPaginas);
}
function irParaPagina(n) {
paginaAtual = n;
aplicarFiltros({ manterPagina: true });
}
function atualizarRodapePaginacao(total, startIdx, sliceLen, totalPaginas) {
const footer = document.getElementById("tableFooter");
const texto = document.getElementById("paginacaoTexto");
const pager = document.getElementById("paginacaoPager");
if (!footer || !texto || !pager) return;
if (total === 0) {
footer.style.display = "none";
return;
}
footer.style.display = "flex";
texto.innerHTML = `Mostrando ${startIdx + 1} a ${startIdx + sliceLen} de ${total} posições
<span style="margin-left:10px;color:var(--muted);">·</span>
<label style="margin-left:10px;display:inline-flex;align-items:center;gap:5px;">
<span>Por página</span>
<select data-on-change="alterarItensPorPagina(this.value)" style="height:28px;border:1px solid var(--border);border-radius:7px;background:var(--bg);color:var(--text);padding:0 6px;font-weight:650;">
<option value="10" ${pageSize===10?'selected':''}>10</option>
<option value="20" ${pageSize===20?'selected':''}>20</option>
<option value="50" ${pageSize===50?'selected':''}>50</option>
</select>
</label>`;
let html = `<button type="button" data-on-click="irParaPagina(${paginaAtual - 1})" ${paginaAtual <= 1 ? "disabled" : ""}>‹</button>`;
// Mantém o rodapé estável: com cinco ou mais páginas, sempre exibimos cinco
// números. Somente o grupo central se desloca conforme a página atual.
const MAX_BOTOES_PAGINA = 5;
const janelas = new Set();
if (totalPaginas <= MAX_BOTOES_PAGINA) {
for (let p = 1; p <= totalPaginas; p++) janelas.add(p);
} else {
const botoesCentrais = MAX_BOTOES_PAGINA - 2;
const inicioCentral = Math.max(
2,
Math.min(paginaAtual - Math.floor(botoesCentrais / 2), totalPaginas - botoesCentrais)
);
janelas.add(1);
for (let p = inicioCentral; p < inicioCentral + botoesCentrais; p++) janelas.add(p);
janelas.add(totalPaginas);
}
let ultimo = 0;
for (let p = 1; p <= totalPaginas; p++) {
if (!janelas.has(p)) continue;
if (p - ultimo > 1) html += `<span class="ellipsis">…</span>`;
html += `<button type="button" class="${p === paginaAtual ? "active" : ""}" data-on-click="irParaPagina(${p})">${p}</button>`;
ultimo = p;
}
html += `<button type="button" data-on-click="irParaPagina(${paginaAtual + 1})" ${paginaAtual >= totalPaginas ? "disabled" : ""}>›</button>`;
pager.innerHTML = html;
}
function renderizarTabela(lista) {
const content = document.getElementById("content");
const cols = quantidadeColunasTabela();
if (lista.length === 0) {
// Durante a inicialização/refresh não mostramos uma tabela vazia.
// Só exibimos "Nenhuma posição" depois que um snapshot vivo foi
// confirmado pela API.
if (carregandoAba && !snapshotAoVivoConfirmado) {
content.innerHTML = `
<table${classeTabelaAtual()}>
${montarThead()}
<tbody><tr class="linha-vazia"><td colspan="${cols}"><div class="estado-vazio carregando"><span class="giro" aria-hidden="true"></span><span>Carregando posições…</span></div></td></tr></tbody>
</table>`;
return;
}
const termoBusca = (document.getElementById("filtroTitulo")?.value || "").trim();
const comFiltro = Boolean(termoBusca || filtroDataDia || filtroCasaAtual !== "todas" || filtroEsportePosicoesAtual !== "todos");
let titulo, texto, acao = "";
if (filtroDataDia) {
const [yy, mm, dd] = filtroDataDia.split("-");
titulo = "Nenhuma aposta em " + dd + "/" + mm + "/" + yy;
texto = "Escolha outro dia ou veja o período inteiro.";
acao = '<button type="button" class="btn" data-on-click="limparFiltroData()">Limpar data</button>';
} else if (comFiltro) {
titulo = "Nada encontrado com estes filtros";
texto = termoBusca ? "Nenhuma posição corresponde a \u201c" + escapeHtml(termoBusca) + "\u201d." : "Ajuste a casa ou o esporte.";
acao = '<button type="button" class="btn" data-on-click="limparBuscaEFiltros()">Limpar filtros</button>';
} else if (abaAtual === "fechado") {
titulo = "Nenhuma aposta fechada ainda";
texto = "As apostas aparecem aqui quando são vendidas ou resolvidas.";
} else if (abaAtual === "ao-vivo") {
titulo = "Nenhum jogo ao vivo agora";
texto = "As posições em jogos que já começaram aparecem aqui.";
} else {
titulo = "Nenhuma posição em aberto";
texto = "Suas apostas em aberto nas casas conectadas aparecem aqui.";
}
content.innerHTML = `
<table${classeTabelaAtual()}>
${montarThead()}
<tbody>
<tr class="linha-vazia">
<td colspan="${cols}"><div class="estado-vazio"><b>${titulo}</b><span>${texto}</span>${acao}</div></td>
</tr>
</tbody>
</table>`;
return;
}
const isAberto = abaAtual === "aberto";
const isAoVivo = abaAtual === "ao-vivo";
let casaAnterior = null;
const rows = lista.map(d => {
const grupo = linhaGrupoCasa(d, casaAnterior, cols);
casaAnterior = casaDaAposta(d);
const pnl = getPnl(d);
const pnlClass = pnl > 0 ? "pnl-positive" : pnl < 0 ? "pnl-negative" : "";
const stake = getStake(d);
const odd = getOdd(d);
const lucroSeVencer = isAoVivo ? getLucroSeVencer(d) : null;
const lucroClass = Number.isFinite(lucroSeVencer) ? "profit-potential" : "";
const lucroLabel = Number.isFinite(lucroSeVencer) ? `${lucroSeVencer >= 0 ? "+" : ""}$${lucroSeVencer.toFixed(2)}` : "—";
const plat = casaDaAposta(d);
const link = plat === "kalshi" ? linkKalshi(d)
: plat === "predict" ? "https://predict.fun/" + (d._marketSlug ? "market/" + encodeURIComponent(d._marketSlug) : "")
: plat === "sxbet" ? "https://sx.bet/"
: (d.eventSlug || d.slug) ? `https://polymarket.com/event/${d.eventSlug || d.slug}` : "#";
const casaLabel = NOMES_CASAS[plat];
const esporteLabel = classificarAposta(d);
const dtInicio = dataPosicaoTabela(d, isAberto || isAoVivo);
const inicioJogo = abaAtual === "fechado" && plat === "predict" && dataInicioAposta(d);
if (dtInicio || inicioJogo) {
renderizarTabela.formatoData ||= new Intl.DateTimeFormat("pt-BR", {
day: "2-digit",
month: "2-digit",
year: "numeric",
hour: "2-digit",
minute: "2-digit",
timeZone: "America/Sao_Paulo"
});
}
const dataCol = dtInicio ? renderizarTabela.formatoData.format(dtInicio) : "—";
const tituloPosicao = tituloExibicao(d) || '—';
const colunasPosicao = BettyMarketTitles.positionColumns(tituloPosicao,d.outcome,d);
const descricaoJogo = inicioJogo ? " · Início " + renderizarTabela.formatoData.format(inicioJogo) : "";
return `${grupo}
<tr>
<td class="col-data" data-label="${abaAtual === 'fechado' ? 'Data de fechamento' : 'Data do evento'}">${dataCol}</td>
<td class="col-casa" data-label="Casa"><img class="casa-logo" src="/assets/platforms/${plat}.png" alt="${escapeHtml(casaLabel)}" title="${escapeHtml(casaLabel)}"></td>
<td class="title-cell col-jogo" data-label="Jogo"><a href="${link}" target="_blank" rel="noopener" title="${escapeHtml(esporteLabel + ' — ' + tituloPosicao + descricaoJogo)}" aria-label="${escapeHtml(esporteLabel + ': ' + colunasPosicao.game + descricaoJogo)}">${iconeCategoria(esporteLabel)}${escapeHtml(colunasPosicao.game)}</a></td>
<td class="title-cell col-aposta" data-label="Aposta"><a href="${link}" target="_blank" rel="noopener" title="${escapeHtml(tituloPosicao)}">${escapeHtml(colunasPosicao.bet)}</a></td>
<td class="col-mid mono" data-label="Odd"${plat === "predict" && d.avgPrice > 0 ? ` title="Preço médio de entrada: ${(100 * d.avgPrice).toFixed(1).replace(".", ",")}¢ por cota"` : ""}>${odd}</td>
<td class="col-mid col-stake mono" data-label="Stake">$${stake.toFixed(2)}</td>
${isAoVivo ? `<td class="col-profit live-profit-cell mono ${lucroClass}" data-label="Profit" title="Lucro líquido estimado se a aposta vencer">${lucroLabel}</td>` : ""}
<td class="col-num pnl-cell mono ${pnlClass}" data-label="PnL">${(isAberto || isAoVivo) && d._platform === "sxbet" && d.cashPnl == null ? "—" : `${pnl >= 0 ? "+" : ""}$${pnl.toFixed(2)}`}</td>
</tr>
`;
}).join("");
content.innerHTML = `
<table${classeTabelaAtual()}>
${montarThead()}
<tbody>
${rows}
</tbody>
</table>
`;
}
function escapeHtml(str) {
return String(str)
.replace(/&/g, "&amp;")
.replace(/</g, "&lt;")
.replace(/>/g, "&gt;")
.replace(/"/g, "&quot;");
}
function getTsFechamento(d) {
// Fallback legado SOMENTE para a conversão histórica de stake.
// Datas de resultados, exportação e sequências usam getTsResultadoPnL().
const ts = d._lastTradeTs || d.timestamp || 0;
if (ts) return ts < 1e12 ? ts * 1000 : ts;
return getDataTs(d) || 0;
}
function getTsPnL(d) {
// Referência histórica da unidade de stake. Não é a data de realização.
// Preservar esta regra evita recalcular valores quando a unidade muda.
if (d._platform === "predict") return BettyAudit.timestampMs(d.timestamp) || 0;
return getDataTs(d) || getTsFechamento(d) || 0;
}
function getTsResultadoPnL(d) {
// Exact market metadata only; event/sibling dates and kickoff never substitute settlement.
const metas = String(d?._platform || 'polymarket').toLowerCase() === 'polymarket' && !d?._resolvedTime ? (metasDatasPnl || (metasDatasPnl = lerCacheMetasDatasGamma())) : null;
const market = metas?.[d?.eventSlug]?.markets?.[d?.slug] || metas?.[d?.slug]?.markets?.[d?.slug];
return BettyAudit.pnlTimestamp(d, market?.closedTime);
}
function fechadosDoGrafico() {
const casa = casaGraficoAtual;
return filtrarExcluidos(fechados).filter(apostaNoPeriodoFechadasConfigurado).filter(d => {
if (casa === "todas") return true;
return String(d?._platform || "polymarket").toLowerCase() === casa;
});
}
function tsPrimeiraApostaFechada() {
let min = Infinity;
fechadosDoGrafico().forEach(d => {
const ts = getTsResultadoPnL(d);
if (ts && ts < min) min = ts;
});
return min === Infinity ? null : min;
}
// Gráficos usam resolução/liquidação ou venda antecipada, nunca início da aposta.
function tsPrimeiroPnL() {
let min = Infinity;
filtrarExcluidos(fechados).filter(apostaNoPeriodoFechadasConfigurado).forEach(d => {
const ts = getTsResultadoPnL(d);
if (ts && ts < min) min = ts;
});
return min === Infinity ? null : min;
}
function chavePnlHistoricoDb() {
const intervalo = periodoGraficoAtual === "estatisticas" ? intervaloPeriodoEstatNoGrafico() : null;
const mode = tipoGraficoAtual === "pnldia" ? "day" : tipoGraficoAtual === "pnlmes" ? "month" : "accumulated";
const period = periodoGraficoAtual === "estatisticas" ? "stats" : periodoGraficoAtual === 7 || periodoGraficoAtual === 30 ? String(periodoGraficoAtual) : "all";
// A chave precisa usar as datas persistidas do filtro, nunca Date.now(). O
// fim aberto do período muda a cada chamada; incluí-lo aqui fazia a resposta
// chegar com uma chave diferente e ser descartada antes de desenhar o gráfico.
const startKey = period === "stats" ? (dataInicioEstat || "") : "";
const endKey = period === "stats" ? (dataFimEstat || "") : "";
return { mode, period, startTs: intervalo?.inicio || 0, endTs: intervalo?.fim || 0, key: [mode, period, casaGraficoAtual, startKey, endKey, chavePeriodoFechadasConfigurado()].join("|") };
}
async function carregarPnlHistoricoDb() {
if (!indiceHistoricoLegivel()) return null;
const cfg = chavePnlHistoricoDb();
const scope=escopoConsultaPosicoes();
const cache=carregarPnlHistoricoDb.cache ||= {scope:'',entries:new Map(),pending:new Map(),bytes:0};
if(cache.scope!==scope){cache.scope=scope;cache.entries.clear();cache.pending.clear();cache.bytes=0;pnlHistoricoDb=null;}
const hit=cache.entries.get(cfg.key);
if(hit){cache.entries.delete(cfg.key);cache.entries.set(cfg.key,hit);pnlHistoricoDb=hit;pnlHistoricoDbEmAndamento=null;return hit.serie;}
const versao = versaoLeituraHistorico();
if(cache.pending.has(cfg.key)){pnlHistoricoDbEmAndamento=cache.pending.get(cfg.key);return pnlHistoricoDbEmAndamento.promise;}
const params = new URLSearchParams({ mode: cfg.mode, period: cfg.period, platform: casaGraficoAtual, now: String(Date.now()), startTs: String(cfg.startTs), endTs: String(cfg.endTs) });
const promise = fetch(urlLeituraHistorico('/api/history/closed/pnl?' + params.toString()), { cache: 'no-store' })
.then(r => { if (!r.ok) throw new Error('PnL HTTP ' + r.status); return r.json(); })
.then(data => {
if (!data?.ok || !Array.isArray(data.serie)) throw new Error('PnL inválido');
if (!resumoHistoricoPodeSerPublicado(versao) || cache.scope!==scope || escopoConsultaPosicoes()!==scope) return null;
const entry={key:cfg.key,serie:data.serie,missingDates:Number(data.missingDates)||0};
const bytes=new TextEncoder().encode(JSON.stringify(data.serie)).byteLength;
if(bytes<=8*1024*1024){
while(cache.entries.size && (cache.entries.size>=6 || cache.bytes+bytes>8*1024*1024)){const oldest=cache.entries.keys().next().value;cache.bytes-=cache.entries.get(oldest).bytes;cache.entries.delete(oldest);}
entry.bytes=bytes;cache.entries.set(cfg.key,entry);cache.bytes+=bytes;
}
if (chavePnlHistoricoDb().key === cfg.key) pnlHistoricoDb = entry;
return data.serie;
}).catch(e => { console.warn('PnL no SQLite indisponível:', e); return null; })
.finally(() => { if(cache.pending.get(cfg.key)?.promise===promise)cache.pending.delete(cfg.key);if(pnlHistoricoDbEmAndamento?.promise===promise)pnlHistoricoDbEmAndamento=null; });
pnlHistoricoDbEmAndamento = { key: cfg.key, versao, promise };
cache.pending.set(cfg.key,pnlHistoricoDbEmAndamento);
return promise;
}
function historicoPnlAcumulado(periodoDias) {
const cfg = chavePnlHistoricoDb();
if (indiceDisponivel() && cfg.mode === "accumulated" && pnlHistoricoDb?.key === cfg.key) return pnlHistoricoDb.serie;
// Série pura de resultado: começa em zero e acumula somente o PnL realizado
// na data de realização. Depósitos, saldo atual e stake aberto não participam.
const eventos = fechadosDoGrafico()
.map(d => ({ t: getTsResultadoPnL(d), v: Number(d.realizedPnl || 0) }))
.filter(p => p.t > 0 && Number.isFinite(p.v))
.sort((a, b) => a.t - b.t);
const agora = Date.now();
const intervaloEstat = periodoDias === "estatisticas" ? intervaloPeriodoEstatNoGrafico() : null;
let corte = intervaloEstat?.inicio ?? corteDoPeriodo(periodoDias);
const fim = intervaloEstat?.fim ?? agora;
if (corte > 0) corte = Math.max(0, corte - 1);
// Em recortes de 7/30 dias, o acumulado começa em zero e considera somente
// os resultados realizados no período selecionado.
let acumulado = 0;
const serie = [];
const inicio = eventos.length ? eventos[0].t - 1 : fim;
if ((intervaloEstat || (periodoDias != null && periodoDias >= 0 && periodoDias < 9999)) && eventos.length) {
serie.push({ t: corte, v: 0 });
for (const ev of eventos) {
if (ev.t >= corte && ev.t <= fim) {
acumulado += ev.v;
serie.push({ t: ev.t, v: acumulado });
}
}
} else {
serie.push({ t: inicio, v: 0 });
for (const ev of eventos) {
acumulado += ev.v;
serie.push({ t: ev.t, v: acumulado });
}
}
// Fecha a curva no acumulado atual, sem reconciliar com saldo de conta.
if (!serie.length || serie[serie.length - 1].t !== fim) {
serie.push({ t: fim, v: acumulado });
} else {
serie[serie.length - 1].v = acumulado;
}
// Evita pontos duplicados no mesmo timestamp, que podem gerar segmentos
// estranhos e uma variação incorreta no cabeçalho.
const limpa = [];
for (const ponto of serie) {
if (limpa.length && limpa[limpa.length - 1].t === ponto.t) limpa[limpa.length - 1] = ponto;
else limpa.push(ponto);
}
return limpa;
}
function sincronizarBotoesPeriodoGrafico() {
const valor = periodoGraficoAtual === "estatisticas" ? "estatisticas" : periodoGraficoAtual === 7 ? "7" : periodoGraficoAtual === 30 ? "30" : "all";
document.querySelectorAll(".chart-period-btn:not(.chart-type-btn)").forEach(b => b.classList.toggle("active", b.dataset.periodo === valor));
}
function trocarTipoGrafico(tipo, btn) {
tipoGraficoAtual = tipo;
document.querySelectorAll(".chart-type-btn").forEach(b => b.classList.toggle("active", b === btn));
const titleEl = document.getElementById("chartTitleText");
const titles = {
patrimonio: "Evolução do PnL Acumulado",
pnldia: "PnL por Dia",
pnlmes: "PnL por Mês",
};
if (titleEl) titleEl.textContent = titles[tipo] || "Gráfico";
// Período só no PnL acumulado
const per = document.getElementById("chartPeriodButtons");
// Mantém o espaço reservado para que o cartão não mude de altura ao trocar
// entre os três gráficos; nos outros tipos os controles apenas ficam ocultos.
if (per) per.style.visibility = tipo === "patrimonio" ? "visible" : "hidden";
// Os gráficos diário e mensal preservam somente o recorte "Período" quando
// ele já estiver selecionado. Os demais atalhos continuam com o comportamento
// anterior de usar todo o histórico nesses dois modos.
if ((tipo === "pnldia" || tipo === "pnlmes") && periodoGraficoAtual !== "estatisticas") periodoGraficoAtual = -1;
desenharGrafico();
}
function trocarPeriodoGrafico(dias) {
if (dias === "estatisticas" && !intervaloPeriodoEstatNoGrafico()) return;
periodoGraficoAtual = dias; // -1 = all time
periodoGraficoPadraoPendente = false;
sincronizarBotoesPeriodoGrafico();
desenharGrafico();
}
function alterarCasaGrafico(casa) {
casaGraficoAtual = CASAS_STAKE.includes(casa) ? casa : "todas";
const labels = { todas: "Todas", ...NOMES_CASAS };
const menu = document.getElementById("graficoCasaMenu");
const trigger = document.getElementById("graficoCasaTrigger");
const valor = document.getElementById("graficoCasaValor");
if (valor) valor.textContent = labels[casaGraficoAtual];
document.querySelectorAll(".chart-house-option").forEach(opcao => {
opcao.classList.toggle("active", opcao.dataset.casa === casaGraficoAtual);
});
if (menu) menu.classList.remove("open");
if (trigger) trigger.setAttribute("aria-expanded", "false");
desenharGrafico();
}
function toggleCasaGraficoMenu() {
const menu = document.getElementById("graficoCasaMenu");
const trigger = document.getElementById("graficoCasaTrigger");
if (!menu || !trigger) return;
const aberto = menu.classList.toggle("open");
trigger.setAttribute("aria-expanded", String(aberto));
}
document.addEventListener("click", event => {
const menu = document.getElementById("graficoCasaMenu");
const trigger = document.getElementById("graficoCasaTrigger");
if (menu && !menu.contains(event.target)) {
menu.classList.remove("open");
if (trigger) trigger.setAttribute("aria-expanded", "false");
}
});
document.addEventListener("keydown", event => {
if (event.key !== "Escape") return;
const menu = document.getElementById("graficoCasaMenu");
const trigger = document.getElementById("graficoCasaTrigger");
if (menu) menu.classList.remove("open");
if (trigger) trigger.setAttribute("aria-expanded", "false");
});
function corteDoPeriodo(periodoDias) {
const agora = Date.now();
if (periodoDias == null || periodoDias < 0 || periodoDias >= 9999) {
const primeira = tsPrimeiraApostaFechada();
return primeira != null ? primeira : 0;
}
let corte = agora - periodoDias * 24 * 60 * 60 * 1000;
if (periodoDias > 7) {
const primeira = tsPrimeiraApostaFechada();
if (primeira != null) corte = Math.max(corte, primeira);
}
return corte;
}
function intervaloPeriodoEstatNoGrafico() {
carregarPeriodoEstatisticas();
if (!dataInicioEstat && !dataFimEstat) return null;
// Sem "De", o SQLite encontra a primeira aposta do universo completo. Não
// usamos a página atual nem o snapshot parcial para definir esse limite.
const inicio = dataInicioEstat ? new Date(`${dataInicioEstat}T00:00:00-03:00`).getTime() : 0;
const fimDoDia = dataFimEstat ? new Date(`${dataFimEstat}T23:59:59.999-03:00`).getTime() : Date.now();
return { inicio, fim: Math.min(Date.now(), fimDoDia) };
}
function atualizarControlePeriodoEstatGrafico(pintar = dashboardEstaVisivel()) {
if (periodoGraficoPadraoPendente || periodoGraficoAtual === "estatisticas") periodoGraficoAtual = -1;
periodoGraficoPadraoPendente = false;
if (pintar) sincronizarBotoesPeriodoGrafico();
}
function desenharGrafico(opcoes = {}) {
const pintar = opcoes.pintar ?? dashboardEstaVisivel();
if (!pintar) dashboardVisualPendente = true;
// Nunca trocamos uma série confirmada por uma leitura intermediária do índice
// nem desenhamos a série com o cache inicial. Se já houver gráfico, ele fica
// intacto até o SQLite e o snapshot vivo confirmarem a nova versão.
if (!dadosFinanceirosConfirmados()) {
if (pintar) {
const area = document.getElementById("chartArea");
if (area && !area.querySelector("svg")) area.innerHTML = '<div class="chart-empty">Sincronizando histórico...</div>';
}
return;
}
const versaoHistorico = versaoLeituraHistorico();
atualizarControlePeriodoEstatGrafico(pintar);
if (indiceDisponivel()) {
const chave = chavePnlHistoricoDb().key;
if (pnlHistoricoDb?.key !== chave) {
carregarPnlHistoricoDb().then(() => {
if (resumoHistoricoPodeSerPublicado(versaoHistorico) && chavePnlHistoricoDb().key === chave) renderizarGraficoComSerie({ pintar });
});
return;
}
}
if (!leituraHistoricoAindaAtual(versaoHistorico)) return;
renderizarGraficoComSerie({ pintar });
}
function renderizarGraficoComSerie(opcoes = {}) {
const pintar = opcoes.pintar ?? dashboardEstaVisivel();
if (!pintar) { dashboardVisualPendente = true; return; }
const notice=document.getElementById('chartDateNotice');
if(notice){const missing=pnlHistoricoDb?.missingDates || 0;notice.hidden=!missing;notice.textContent=missing ? `${missing} apostas ainda sem data de realização confirmada. Seus resultados não estão nesta curva; o total pode diferir até recuperar as datas.` : '';}
if (tipoGraficoAtual === "pnldia") desenharGraficoPnlDia();
else if (tipoGraficoAtual === "pnlmes") desenharGraficoPnlMes();
else desenharGraficoPatrimonio();
dashboardVisualPendente = false;
}
// Catmull-Rom → Bézier: curva suave passando exatamente pelos pontos reais
function curvaSuave(coords) {
if (coords.length < 3) {
return coords.map((c, i) => (i === 0 ? "M" : "L") + c[0].toFixed(2) + "," + c[1].toFixed(2)).join(" ");
}
let d = `M ${coords[0][0].toFixed(2)},${coords[0][1].toFixed(2)}`;
for (let i = 0; i < coords.length - 1; i++) {
const p0 = coords[i - 1] || coords[i];
const p1 = coords[i];
const p2 = coords[i + 1];
const p3 = coords[i + 2] || p2;
const c1x = p1[0] + (p2[0] - p0[0]) / 6;
const c1y = p1[1] + (p2[1] - p0[1]) / 6;
const c2x = p2[0] - (p3[0] - p1[0]) / 6;
const c2y = p2[1] - (p3[1] - p1[1]) / 6;
d += ` C ${c1x.toFixed(2)},${c1y.toFixed(2)} ${c2x.toFixed(2)},${c2y.toFixed(2)} ${p2[0].toFixed(2)},${p2[1].toFixed(2)}`;
}
return d;
}
// Curva de Bézier monotônica: passa pelos pontos reais e evita ultrapassar
// máximos/mínimos entre dois dias consecutivos.
function curvaSuaveMonotona(coords) {
if (coords.length < 2) return coords.length ? `M${coords[0][0].toFixed(2)},${coords[0][1].toFixed(2)}` : "";
const n = coords.length;
const h = [], delta = [];
for (let i = 0; i < n - 1; i++) {
const dx = coords[i + 1][0] - coords[i][0];
h.push(dx);
delta.push(dx ? (coords[i + 1][1] - coords[i][1]) / dx : 0);
}
const tangentes = new Array(n);
tangentes[0] = delta[0];
tangentes[n - 1] = delta[n - 2];
for (let i = 1; i < n - 1; i++) {
const anterior = delta[i - 1], proxima = delta[i];
if (anterior === 0 || proxima === 0 || anterior * proxima < 0) tangentes[i] = 0;
else {
const w1 = 2 * h[i] + h[i - 1];
const w2 = h[i] + 2 * h[i - 1];
tangentes[i] = (w1 + w2) / (w1 / anterior + w2 / proxima);
}
}
let d = `M${coords[0][0].toFixed(2)},${coords[0][1].toFixed(2)}`;
for (let i = 0; i < n - 1; i++) {
const p1 = coords[i], p2 = coords[i + 1], dx = h[i] / 3;
d += ` C${(p1[0] + dx).toFixed(2)},${(p1[1] + tangentes[i] * dx).toFixed(2)} ${(p2[0] - dx).toFixed(2)},${(p2[1] - tangentes[i + 1] * dx).toFixed(2)} ${p2[0].toFixed(2)},${p2[1].toFixed(2)}`;
}
return d;
}
// A série lógica permanece intacta. Para o acumulado exibido, cada data de
// São Paulo usa o último valor cronológico — o PnL acumulado ao fim do dia.
function agruparPnlAcumuladoPorDia(pontos) {
const porDia = new Map();
for (const ponto of pontos) {
const dia = ymdBrasil(new Date(ponto.t));
if (!dia) continue;
// Como a série já vem em ordem cronológica, substituir preserva o último
// movimento real do dia e também trata corretamente os pontos técnicos.
porDia.set(dia, { ...ponto, dia });
}
return [...porDia.values()];
}
function interpolarCorPnl(inicio, fim, proporcao) {
const t = Math.max(0, Math.min(1, proporcao));
const a = inicio.match(/[a-f\d]{2}/gi).map(hex => parseInt(hex, 16));
const b = fim.match(/[a-f\d]{2}/gi).map(hex => parseInt(hex, 16));
return "#" + a.map((canal, i) => Math.round(canal + (b[i] - canal) * t).toString(16).padStart(2, "0")).join("");
}
// A cor é relativa à amplitude visual do período: vermelho → coral → roxo
// → azul → verde. Nenhum valor da série é alterado por esta apresentação.
function corGradientePnl(valor, amplitude) {
const escala = Number(amplitude) > 0 ? Number(amplitude) : 1;
const posicao = Math.max(-1, Math.min(1, (Number(valor) || 0) / escala));
if (posicao <= -0.55) return interpolarCorPnl("#ef4444", "#fb7185", (posicao + 1) / 0.45);
if (posicao < 0) return interpolarCorPnl("#fb7185", "#7c3aed", (posicao + 0.55) / 0.55);
if (posicao < 0.55) return interpolarCorPnl("#7c3aed", "#38bdf8", posicao / 0.55);
return interpolarCorPnl("#38bdf8", "#22c55e", (posicao - 0.55) / 0.45);
}
function stopsGradientePnl(pontosVisuais, pontosDiarios, amplitude) {
if (pontosDiarios.length === 1) {
const cor = corGradientePnl(pontosDiarios[0].v, amplitude);
return `<stop offset="0%" stop-color="${cor}"/><stop offset="100%" stop-color="${cor}"/>`;
}
return pontosVisuais.map(({ ponto, index }) => {
const offset = ((index / (pontosDiarios.length - 1)) * 100).toFixed(3);
return `<stop offset="${offset}%" stop-color="${corGradientePnl(ponto.v, amplitude)}"/>`;
}).join("");
}
// A série original nunca é alterada. Em históricos grandes, esta projeção é
// usada exclusivamente para reduzir o custo de construir o path SVG: cada
// faixa preserva os extremos (na ordem temporal), além do primeiro e último.
const LIMITE_PONTOS_VISUAIS_GRAFICO = 1200;
function pontosVisuaisGraficoAcumulado(pontos, limite = LIMITE_PONTOS_VISUAIS_GRAFICO) {
if (pontos.length <= limite) return pontos.map((ponto, index) => ({ ponto, index }));
const porFaixa = Math.max(1, Math.floor(limite / 2));
const faixa = Math.ceil(pontos.length / porFaixa);
const indices = new Set([0, pontos.length - 1]);
for (let inicio = 0; inicio < pontos.length; inicio += faixa) {
const fim = Math.min(pontos.length, inicio + faixa);
let menor = inicio, maior = inicio;
for (let i = inicio + 1; i < fim; i++) {
if (pontos[i].v < pontos[menor].v) menor = i;
if (pontos[i].v > pontos[maior].v) maior = i;
}
indices.add(menor);
indices.add(maior);
}
return [...indices].sort((a, b) => a - b).map(index => ({ ponto: pontos[index], index }));
}
function fmtDinheiro(v) {
const abs = Math.abs(v);
return (v < 0 ? "-$" : "$") + abs.toFixed(abs >= 1000 ? 0 : 2);
}
// ===== Exportação CSV =====
function fecharExportPopup(retornarFoco = false) {
const popup = document.getElementById("csvExportPopup");
const trigger = document.querySelector(".btn-download-csv");
if (!popup || popup.hidden) return;
popup.hidden = true;
trigger?.setAttribute("aria-expanded", "false");
if (retornarFoco) trigger?.focus();
}
function toggleExportPopup(ev) {
ev?.stopPropagation();
const popup = document.getElementById("csvExportPopup");
const trigger = document.querySelector(".btn-download-csv");
if (!popup || !trigger) return;
if (!popup.hidden) { fecharExportPopup(true); return; }
fecharPopupCalendario();
popup.hidden = false;
trigger.setAttribute("aria-expanded", "true");
posicionarExportPopup();
document.getElementById("escopoCSV")?.focus({ preventScroll: true });
}
function posicionarExportPopup() {
const popup = document.getElementById("csvExportPopup");
const trigger = document.querySelector(".btn-download-csv");
if (!popup || popup.hidden || !trigger) return;
const rect = trigger.getBoundingClientRect();
if (rect.bottom < 0 || rect.top > window.innerHeight) { fecharExportPopup(); return; }
popup.style.left = `${Math.max(8, Math.min(rect.right - popup.offsetWidth, window.innerWidth - popup.offsetWidth - 8))}px`;
popup.style.top = `${Math.max(8, Math.min(rect.bottom + 8, window.innerHeight - popup.offsetHeight - 8))}px`;
}
document.addEventListener("click", ev => {
if (!ev.target.closest(".csv-export-wrap")) fecharExportPopup();
});
document.addEventListener("keydown", ev => {
if (ev.key === "Escape") fecharExportPopup(true);
});
window.addEventListener("resize", posicionarExportPopup);
document.addEventListener("scroll", posicionarExportPopup, true);
function csvEsc(v) {
const s = String(v == null ? "" : v);
return /[";,\n\r]/.test(s) ? '"' + s.replace(/"/g, '""') + '"' : s;
}
function timestampExportacaoPosicao(d) {
return d._statusExport === 'Fechada'
? getTsResultadoPnL(d)
: BettyAudit.timestampMs(d.timestamp || getTsFechamento(d));
}
function formatarDataCSV(ts) {
return BettyAudit.csvDate(ts);
}
function tituloExibicaoCSV(d) {
try {
return tituloExibicao(d) || d?.title || "";
} catch (erro) {
// A normalização do título é apenas apresentação: uma linha com metadados
// inesperados nunca deve impedir a exportação de todas as outras apostas.
console.warn("Título não normalizado na exportação CSV:", erro);
return String(d?.title || "");
}
}
// Chamado pelo backend (on_download) quando um download da interface termina.
// Mostra onde o arquivo foi salvo; o texto entra só como textContent.
let temporizadorAvisoDownload = null;
function avisarDownload(info) {
const aviso = document.getElementById("avisoDownload");
if (!aviso) return;
const ok = Boolean(info?.ok);
aviso.textContent = ok
? `CSV salvo em ${info.pasta || "Downloads"}: ${info.nome || ""}`
: "Não foi possível salvar o CSV. Tente exportar novamente.";
aviso.classList.toggle("erro", !ok);
aviso.hidden = false;
clearTimeout(temporizadorAvisoDownload);
temporizadorAvisoDownload = setTimeout(() => { aviso.hidden = true; }, 8000);
}
async function baixarCSVPosicoes() {
const botao = document.querySelector(".csv-export-confirm");
if (botao?.disabled) return;
const rotuloOriginal = botao?.getAttribute("aria-label") || "Baixar CSV";
if (botao) {
botao.disabled = true;
botao.setAttribute("aria-label", "Preparando download do CSV...");
}
try {
// O escopo selecionado é explícito; "todo o histórico" ignora filtros de período.
// Não deduplicamos por mercado, ticker, conditionId ou outcome.
// Isso permite preservar múltiplas apostas/operações no mesmo mercado e
// também manter um registro caso a API exponha a mesma posição em mais
// de uma coleção durante a transição de aberta para fechada.
const scope = document.getElementById('escopoCSV')?.value || 'all';
const exportedAt = new Date().toISOString();
const scopeLabel = { all: 'Todo o histórico carregado; inclui manuais e antigas', periods: 'Períodos por casa; abertas sem filtro de período', stats: 'Somente fechadas; mesmos filtros das estatísticas' }[scope];
const filters = scope === 'all' ? {} : {
periodosPorCasa: PERIODO_FECHADAS_CONFIGURADO,
...(scope === 'stats' ? { casa: filtroCasaEstat, categoria: filtroEsporteEstat, oddsRange: filtroOddsEstat, inicio: dataInicioEstat, fim: dataFimEstat } : {})
};
const apostasAbertas = (scope === 'stats' ? [] : (abertos || [])).map((d, i) => ({
...d,
_statusExport: "Aberta",
_exportIndex: i
}));
// A exportação sempre lê o ledger completo do SQLite. A tabela pode estar na
// página 1 com 20 linhas, mas isso nunca reduz o CSV.
const respostaFechadas = await fetch('/api/history/closed/export', { cache: 'no-store' });
if (!respostaFechadas.ok) throw new Error('exportação do histórico HTTP ' + respostaFechadas.status);
const dadosFechados = await respostaFechadas.json();
if (!dadosFechados?.ok || !Array.isArray(dadosFechados.fechados)) throw new Error('exportação do histórico inválida');
const selecionadas = BettyAudit.selectExport({ open: [], closed: dadosFechados.fechados, scope,
housePeriod: apostaNoPeriodoFechadasConfigurado,
statsFilter: d => (filtroEsporteEstat === 'todos' || classificarAposta(d) === filtroEsporteEstat) &&
(filtroCasaEstat === 'todas' || casaDaAposta(d) === filtroCasaEstat) && correspondePeriodoEstat(d) && correspondeOddsEstat(d)
});
const apostasFechadas = selecionadas.closed.map((d, i) => ({
...d,
_statusExport: "Fechada",
_exportIndex: i
}));
const rows = apostasAbertas.concat(apostasFechadas);
if (!rows.length) {
alert("Ainda não há apostas carregadas para exportar.");
return;
}
rows.sort((a,b) => {
const ta = timestampExportacaoPosicao(a) || 0;
const tb = timestampExportacaoPosicao(b) || 0;
return tb - ta;
});
const header = [
"ID do Registro","Status","Data","Plataforma","Categoria",
"Mercado","Resultado","Odd","Stake","P&L","ROI","Ticker/ID",
"Data ISO UTC","Exportado em UTC","Escopo","Filtros","Unidade do registro","Fonte do P&L",
"Taxa conhecida","Lucro não realizado","Última sincronização da casa UTC","Histórico lido em UTC"
];
const lines = [header.map(csvEsc).join(";")];
rows.forEach((d, index) => {
const stake = Number(getStake(d)) || 0;
const pnl = Number(d.realizedPnl ?? d.cashPnl ?? 0) || 0;
const roi = stake > 0 ? (pnl / stake) * 100 : 0;
const tradeId =
d.tradeId || d.trade_id || d.id || d.asset ||
d.conditionId || d.slug ||
((d._platform || "polymarket") + "-" + d._exportIndex + "-" + index);
lines.push([
tradeId,
d._statusExport || "",
formatarDataCSV(timestampExportacaoPosicao(d)),
NOMES_CASAS[casaDaAposta(d)],
classificarAposta(d),
tituloExibicaoCSV(d),
d.outcome || "",
Number(getOddNum(d) || 0).toFixed(4),
stake.toFixed(4),
d._statusExport === 'Fechada' ? pnl.toFixed(4) : '',
d._statusExport === 'Fechada' ? roi.toFixed(4) + '%' : '',
(d._platform === 'predict' || d._platform === 'sxbet' ? d.conditionId : d.slug || d.conditionId) || '',
BettyAudit.isoDate(timestampExportacaoPosicao(d)), exportedAt, scopeLabel, JSON.stringify(filters),
'Posição por mercado/lado; pode agregar operações',
d._statusExport === 'Aberta' ? 'Não realizado' : d._pnlFluxo ? 'Polymarket activity completo' : ({ kalshi: 'Kalshi settlements', predict: 'Predict.fun negociações e resgates', sxbet: 'SX Bet posições liquidadas' }[casaDaAposta(d)] || 'Polymarket positions'),
d._feeCost == null ? '' : String(d._feeCost),
d._statusExport === 'Aberta' && d.cashPnl != null ? String(d.cashPnl) : '',
syncExportAt[casaDaAposta(d)] || '', dadosFechados.exportedAt || ''
].map(csvEsc).join(";"));
});
const nomeArquivo = `betty-apostas-${scope}-${new Date().toISOString().slice(0,10)}.csv`;
const conteudoCsv = "\ufeff" + lines.join("\r\n");
const blob = new Blob([conteudoCsv], { type:"text/csv;charset=utf-8;" });
// Mantém compatibilidade com navegadores baseados em Edge antigos. No app, o
// link temporário abaixo inicia o download e o backend (on_download) salva o
// arquivo na pasta Downloads e chama avisarDownload() ao terminar.
if (navigator.msSaveOrOpenBlob) {
navigator.msSaveOrOpenBlob(blob, nomeArquivo);
return;
}
const url = URL.createObjectURL(blob);
const a = document.createElement("a");
a.href = url;
a.download = nomeArquivo;
a.style.display = "none";
document.body.appendChild(a);
// O clique ocorre no mesmo gesto do usuário, necessário para o Chromium
// (WebView2) aceitar downloads programáticos.
a.click();
a.remove();
setTimeout(() => URL.revokeObjectURL(url), 30000);
} catch (erro) {
console.error("Falha ao exportar apostas em CSV:", erro);
alert("Não foi possível gerar o CSV. Tente atualizar os dados e exportar novamente.");
} finally {
if (botao) {
botao.disabled = false;
botao.setAttribute("aria-label", rotuloOriginal);
}
}
}
// ===== Estatísticas =====
let subAbaEstat = "site";
const CORES_SITE = { polymarket: "#3b82f6", kalshi: "#22c55e", predict: "#c084fc", sxbet: "#fbbf24" };
// ===== Classificação de mercados =====
// Toda aposta recebe exatamente uma categoria. Esportes são quebrados por
// modalidade; o restante do livro (política, cripto, tecnologia, games...)
// entra pelo tema do mercado. Nada fica de fora: sem evidência, cai em "Outros".
const CORES_CATEGORIA = {
"Futebol": "#22c55e",
"Basquete": "#f59e0b",
"Futebol Americano": "#3b82f6",
"Beisebol": "#ef4444",
"Hóquei": "#06b6d4",
"Tênis": "#84cc16",
"MMA & Boxe": "#f97316",
"Golfe": "#10b981",
"Automobilismo": "#e11d48",
"Esports": "#8b5cf6",
"Críquete": "#0d9488",
"Rugby": "#65a30d",
"Vôlei": "#facc15",
"Outros Esportes": "#94a3b8",
"Política": "#6366f1",
"Geopolítica": "#0ea5e9",
"Economia": "#eab308",
"Cripto": "#f7931a",
"Tecnologia & IA": "#d946ef",
"Games": "#ec4899",
"Entretenimento": "#fb7185",
"Ciência & Saúde": "#2dd4bf",
"Clima": "#38bdf8",
"Outros": "#6b7280"
};
const CATEGORIAS_ESPORTE = ["Futebol", "Basquete", "Futebol Americano", "Beisebol", "Hóquei", "Tênis", "MMA & Boxe", "Golfe", "Automobilismo", "Esports", "Críquete", "Rugby", "Vôlei", "Outros Esportes"];
const CATEGORIAS_TEMA = ["Política", "Geopolítica", "Economia", "Cripto", "Tecnologia & IA", "Games", "Entretenimento", "Ciência & Saúde", "Clima", "Outros"];
const ORDEM_CATEGORIA = [...CATEGORIAS_ESPORTE, ...CATEGORIAS_TEMA];
const CATEGORIA_OUTROS = "Outros";
const ESPORTE_GENERICO = "Outros Esportes";
function corDaCategoria(nome) { return CORES_CATEGORIA[nome] || "#6b7280"; }
function ehCategoriaEsporte(nome) { return CATEGORIAS_ESPORTE.includes(nome); }
let filtroEsporteEstat = "todos";
let filtroCasaEstat = "todas";
let filtroOddsEstat = "todas";
let atalhoPeriodoEstat = "tudo";
let dataInicioEstat = "";
let dataFimEstat = "";
const STATS_PERIOD_STORAGE_PREFIX = "betty:statsPeriod:";
let periodoEstatCarregadoPara = null;
let calendarioPeriodoEstatMes = new Date();
let calendarioPeriodoEstatAberto = false;
let campoCalendarioPeriodoEstat = "start";

function chavePeriodoEstat() { return STATS_PERIOD_STORAGE_PREFIX + (BETTY_USER?.id || "local"); }
function carregarPeriodoEstatisticas() {
const chave = chavePeriodoEstat();
if (periodoEstatCarregadoPara === chave) return;
periodoEstatCarregadoPara = chave;
try {
const salvo = JSON.parse(localStorage.getItem(chave) || "null");
dataInicioEstat = /^\d{4}-\d{2}-\d{2}$/.test(salvo?.inicio || "") ? salvo.inicio : "";
dataFimEstat = /^\d{4}-\d{2}-\d{2}$/.test(salvo?.fim || "") ? salvo.fim : "";
filtroCasaEstat = CASAS_STAKE.includes(salvo?.casa) ? salvo.casa : "todas";
filtroOddsEstat = BettyOddsRanges.get(salvo?.oddsRange) ? salvo.oddsRange : "todas";
filtroEsporteEstat = typeof salvo?.categoria === "string" && salvo.categoria ? salvo.categoria : "todos";
atalhoPeriodoEstat = salvo?.atalho || inferirAtalhoPeriodoEstat();
} catch (_) { dataInicioEstat = ""; dataFimEstat = ""; }
}
function salvarPeriodoEstatisticas() {
try {
const chave = chavePeriodoEstat();
localStorage.setItem(chave, JSON.stringify({
inicio: dataInicioEstat, fim: dataFimEstat, casa: filtroCasaEstat,
categoria: filtroEsporteEstat, oddsRange: filtroOddsEstat, atalho: atalhoPeriodoEstat
}));
} catch (_) {}
}
function inferirAtalhoPeriodoEstat() {
if (!dataInicioEstat && !dataFimEstat) return "tudo";
const hoje = ymdBrasil(new Date());
if (dataInicioEstat === hoje && dataFimEstat === hoje) return "hoje";
if (dataInicioEstat === ymdOffset(-6) && dataFimEstat === hoje) return "7";
if (dataInicioEstat === ymdOffset(-29) && dataFimEstat === hoje) return "30";
const mesAtual = hoje.slice(0, 7);
if (dataInicioEstat === mesAtual + "-01" && dataFimEstat === hoje) return "mes";
if (dataInicioEstat === hoje.slice(0, 4) + "-01-01" && dataFimEstat === hoje) return "ano";
return "custom";
}
function primeiroDiaMes(offsetMes = 0) {
const hoje = ymdBrasil(new Date());
const [y, m] = hoje.split("-").map(Number);
const dt = new Date(Date.UTC(y, m - 1 + offsetMes, 1));
return `${dt.getUTCFullYear()}-${String(dt.getUTCMonth() + 1).padStart(2, "0")}-01`;
}
function ultimoDiaMes(offsetMes = 0) {
const ini = primeiroDiaMes(offsetMes);
const [y, m] = ini.split("-").map(Number);
const dt = new Date(Date.UTC(y, m, 0));
return `${dt.getUTCFullYear()}-${String(dt.getUTCMonth() + 1).padStart(2, "0")}-${String(dt.getUTCDate()).padStart(2, "0")}`;
}
function aplicarAtalhoPeriodoEstat(atalho) {
atalhoPeriodoEstat = atalho;
const hoje = ymdBrasil(new Date());
if (atalho === "hoje") { dataInicioEstat = hoje; dataFimEstat = hoje; }
else if (atalho === "ano") { dataInicioEstat = hoje.slice(0, 4) + "-01-01"; dataFimEstat = hoje; }
else if (atalho === "7") { dataInicioEstat = ymdOffset(-6); dataFimEstat = hoje; }
else if (atalho === "30") { dataInicioEstat = ymdOffset(-29); dataFimEstat = hoje; }
else if (atalho === "mes") { dataInicioEstat = primeiroDiaMes(0); dataFimEstat = hoje; }
else if (atalho === "mesant") { dataInicioEstat = primeiroDiaMes(-1); dataFimEstat = ultimoDiaMes(-1); }
else { dataInicioEstat = ""; dataFimEstat = ""; atalhoPeriodoEstat = "tudo"; }
salvarPeriodoEstatisticas();
periodoGraficoPadraoPendente = true;
sincronizarPeriodoEstatisticas();
renderEstatisticas();
if (typeof desenharGrafico === "function") desenharGrafico();
}
function selecionarAtalhoPeriodoEstat(atalho) {
if (atalho !== "custom") { aplicarAtalhoPeriodoEstat(atalho); return; }
atalhoPeriodoEstat = "custom";
salvarPeriodoEstatisticas();
sincronizarPeriodoEstatisticas();
renderEstatisticas();
if (typeof desenharGrafico === "function") desenharGrafico();
}
function alterarFiltroCasaEstat(valor) {
filtroCasaEstat = valor || "todas";
salvarPeriodoEstatisticas();
renderEstatisticas();
}
function alterarFiltroOddsEstat(valor) {
filtroOddsEstat = BettyOddsRanges.get(valor) ? valor : "todas";
salvarPeriodoEstatisticas();
renderEstatisticas();
}
function correspondeOddsEstat(d) {
return BettyOddsRanges.matches(getOddNum(d), filtroOddsEstat);
}
function atualizarAtalhosPeriodoUI() {
const periodo = document.getElementById("statsPeriodSelect");
if (periodo) periodo.value = atalhoPeriodoEstat || "tudo";
const datas = document.querySelector(".stats-period-filter");
const exibirDatas = (atalhoPeriodoEstat || "tudo") !== "tudo";
if (datas) datas.hidden = !exibirDatas;
if (!exibirDatas && calendarioPeriodoEstatAberto) fecharCalendarioPeriodoEstat();
const casa = document.getElementById("statsHouseFilter");
if (casa) casa.value = filtroCasaEstat || "todas";
const odds = document.getElementById("statsOddsFilter");
if (odds) {
if (odds.options.length === 1) odds.innerHTML += BettyOddsRanges.ranges.map(r => `<option value="${r.id}">${r.label}</option>`).join('');
odds.value = filtroOddsEstat;
}
}
function renderFiltrosAtivosEstat() {
const el = document.getElementById("statsActiveFilters");
if (!el) return;
const chips = [];
const rotuloAtalho = { hoje: "Hoje", "7": "7 dias", "30": "30 dias", mes: "Mês atual", mesant: "Mês anterior", ano: "Ano atual", tudo: "Tudo", custom: "Período customizado" };
if (atalhoPeriodoEstat && atalhoPeriodoEstat !== "tudo") chips.push({ k: "Período", v: rotuloAtalho[atalhoPeriodoEstat] || (dataInicioEstat + " → " + dataFimEstat), limpar: () => aplicarAtalhoPeriodoEstat("tudo") });
else if (dataInicioEstat || dataFimEstat) chips.push({ k: "Período", v: (dataInicioEstat || "…") + " → " + (dataFimEstat || "…"), limpar: () => aplicarAtalhoPeriodoEstat("tudo") });
if (filtroCasaEstat && filtroCasaEstat !== "todas") chips.push({ k: "Casa", v: NOMES_CASAS[filtroCasaEstat], limpar: () => alterarFiltroCasaEstat("todas") });
if (filtroEsporteEstat && filtroEsporteEstat !== "todos") chips.push({ k: "Categoria", v: filtroEsporteEstat, limpar: () => alterarFiltroEsporte("todos") });
if (BettyOddsRanges.get(filtroOddsEstat)) chips.push({ k: "Odds", v: BettyOddsRanges.get(filtroOddsEstat).label, limpar: () => alterarFiltroOddsEstat("todas") });
if (!chips.length) { el.classList.remove("show"); el.innerHTML = ""; return; }
el.classList.add("show");
el.innerHTML = `<span>Filtros:</span>` + chips.map(c => `<span class="filter-chip">${escapeHtml(c.k)}: ${escapeHtml(c.v)}<button type="button" aria-label="Remover ${escapeHtml(c.k)}">×</button></span>`).join("");
el.querySelectorAll("button").forEach((btn, i) => { btn.onclick = chips[i].limpar; });
}
function sincronizarPeriodoEstatisticas() {
carregarPeriodoEstatisticas();
// Atalhos relativos acompanham o calendário mesmo após reabrir o app.
if (atalhoPeriodoEstat === "hoje" || atalhoPeriodoEstat === "ano") {
const hoje = ymdBrasil(new Date());
dataInicioEstat = atalhoPeriodoEstat === "hoje" ? hoje : hoje.slice(0, 4) + "-01-01";
dataFimEstat = hoje;
}
atualizarAtalhosPeriodoUI();
renderFiltrosAtivosEstat();
const inicio = document.getElementById("statsDateStart");
const fim = document.getElementById("statsDateEnd");
if (inicio) { inicio.value = dataInicioEstat; inicio.max = dataFimEstat || ""; }
if (fim) { fim.value = dataFimEstat; fim.min = dataInicioEstat || ""; }
const texto = data => data ? data.split("-").reverse().join("/") : "dd/mm/aaaa";
for (const [campo, data] of [["Start", dataInicioEstat], ["End", dataFimEstat]]) {
const valor = document.getElementById(`statsDate${campo}Value`);
const botao = document.getElementById(`statsDate${campo}Button`);
if (valor) valor.textContent = texto(data);
if (botao) botao.classList.toggle("active", Boolean(data));
}
}

function toggleCalendarioPeriodoEstat(campo, ev) {
if (ev) ev.stopPropagation();
const popup = document.getElementById("statsDatePopup");
if (!popup) return;
if (calendarioPeriodoEstatAberto && campoCalendarioPeriodoEstat === campo) {
fecharCalendarioPeriodoEstat();
return;
}
campoCalendarioPeriodoEstat = campo;
const selecionada = campo === "start" ? dataInicioEstat : dataFimEstat;
if (selecionada) {
const [ano, mes] = selecionada.split("-").map(Number);
calendarioPeriodoEstatMes = new Date(ano, mes - 1, 1);
} else {
const hoje = new Date();
calendarioPeriodoEstatMes = new Date(hoje.getFullYear(), hoje.getMonth(), 1);
}
renderCalendarioPeriodoEstat();
calendarioPeriodoEstatAberto = true;
popup.classList.add("open");
document.getElementById(`statsDate${campo === "start" ? "Start" : "End"}Button`)?.setAttribute("aria-expanded", "true");
requestAnimationFrame(posicionarCalendarioPeriodoEstat);
document.addEventListener("scroll", fecharCalendarioPeriodoEstat, true);
setTimeout(() => document.addEventListener("click", fecharCalendarioPeriodoEstatFora), 0);
}
function fecharCalendarioPeriodoEstatFora(ev) {
const popup = document.getElementById("statsDatePopup");
const botao = document.getElementById(`statsDate${campoCalendarioPeriodoEstat === "start" ? "Start" : "End"}Button`);
if (popup && !popup.contains(ev.target) && botao && !botao.contains(ev.target)) fecharCalendarioPeriodoEstat();
}
function fecharCalendarioPeriodoEstat() {
const popup = document.getElementById("statsDatePopup");
if (popup) { popup.classList.remove("open"); popup.style.top = ""; popup.style.left = ""; }
document.getElementById("statsDateStartButton")?.setAttribute("aria-expanded", "false");
document.getElementById("statsDateEndButton")?.setAttribute("aria-expanded", "false");
calendarioPeriodoEstatAberto = false;
document.removeEventListener("click", fecharCalendarioPeriodoEstatFora);
document.removeEventListener("scroll", fecharCalendarioPeriodoEstat, true);
}
function posicionarCalendarioPeriodoEstat() {
const popup = document.getElementById("statsDatePopup");
const botao = document.getElementById(`statsDate${campoCalendarioPeriodoEstat === "start" ? "Start" : "End"}Button`);
if (!popup || !botao || !popup.classList.contains("open")) return;
const r = botao.getBoundingClientRect();
const margem = 8, espacamento = 8, largura = popup.offsetWidth || 260, altura = popup.offsetHeight || 330;
let esquerda = r.left, topo = r.bottom + espacamento;
if (esquerda + largura > window.innerWidth - margem) esquerda = window.innerWidth - largura - margem;
if (esquerda < margem) esquerda = margem;
if (topo + altura > window.innerHeight - margem) topo = Math.max(margem, r.top - altura - espacamento);
popup.style.left = `${esquerda}px`;
popup.style.top = `${topo}px`;
}
function navegarMesCalendarioPeriodoEstat(delta, ev) {
if (ev) ev.stopPropagation();
calendarioPeriodoEstatMes = new Date(calendarioPeriodoEstatMes.getFullYear(), calendarioPeriodoEstatMes.getMonth() + delta, 1);
renderCalendarioPeriodoEstat();
}
function renderCalendarioPeriodoEstat() {
const popup = document.getElementById("statsDatePopup");
if (!popup) return;
const ano = calendarioPeriodoEstatMes.getFullYear(), mes = calendarioPeriodoEstatMes.getMonth();
const nomesMes = ["janeiro", "fevereiro", "março", "abril", "maio", "junho", "julho", "agosto", "setembro", "outubro", "novembro", "dezembro"];
const primeiroDia = new Date(ano, mes, 1).getDay(), diasNoMes = new Date(ano, mes + 1, 0).getDate(), diasMesAnterior = new Date(ano, mes, 0).getDate();
const pad2 = n => String(n).padStart(2, "0"), hoje = ymdBrasil(new Date());
const selecionada = campoCalendarioPeriodoEstat === "start" ? dataInicioEstat : dataFimEstat;
let celulas = "";
for (let i = primeiroDia - 1; i >= 0; i--) celulas += `<div class="day other">${diasMesAnterior - i}</div>`;
for (let dia = 1; dia <= diasNoMes; dia++) {
const ymd = `${ano}-${pad2(mes + 1)}-${pad2(dia)}`;
let classe = "day";
if (ymd === hoje) classe += " today";
if (ymd === selecionada) classe += " selected";
celulas += `<div class="${classe}" data-on-click="selecionarDiaPeriodoEstat('${ymd}', event)">${dia}</div>`;
}
const resto = (primeiroDia + diasNoMes) % 7;
if (resto) for (let dia = 1; dia <= 7 - resto; dia++) celulas += `<div class="day other">${dia}</div>`;
popup.innerHTML = `<div class="date-popup-head"><span>${nomesMes[mes]} de ${ano}</span><div class="date-popup-nav"><button type="button" data-on-click="navegarMesCalendarioPeriodoEstat(-1,event)">‹</button><button type="button" data-on-click="navegarMesCalendarioPeriodoEstat(1,event)">›</button></div></div><div class="date-grid"><div class="dow">D</div><div class="dow">S</div><div class="dow">T</div><div class="dow">Q</div><div class="dow">Q</div><div class="dow">S</div><div class="dow">S</div>${celulas}</div><div class="date-popup-foot"><button type="button" class="limpar" data-on-click="limparCampoPeriodoEstat(event)">Limpar</button><button type="button" data-on-click="selecionarDiaPeriodoEstat('${hoje}',event)">Hoje</button></div>`;
}
function selecionarDiaPeriodoEstat(ymd, ev) {
if (ev) ev.stopPropagation();
const input = document.getElementById(campoCalendarioPeriodoEstat === "start" ? "statsDateStart" : "statsDateEnd");
if (input) input.value = ymd;
fecharCalendarioPeriodoEstat();
alterarPeriodoEstatisticas();
}
function limparCampoPeriodoEstat(ev) {
if (ev) ev.stopPropagation();
const input = document.getElementById(campoCalendarioPeriodoEstat === "start" ? "statsDateStart" : "statsDateEnd");
if (input) input.value = "";
fecharCalendarioPeriodoEstat();
alterarPeriodoEstatisticas();
}

function chaveDataInicioEstat(d) {
// Mesmo dia de realização exibido em Posições > Fechadas e nos gráficos.
// Não modifica a data original do evento nem a conversão histórica de stake.
const data = dataPosicaoTabela(d, false);
if (!data) return "";
chaveDataInicioEstat.formato ||= new Intl.DateTimeFormat("en-US", {
timeZone: "America/Sao_Paulo", year: "numeric", month: "2-digit", day: "2-digit"
});
const partes = chaveDataInicioEstat.formato.formatToParts(data);
const valor = tipo => partes.find(p => p.type === tipo)?.value || "";
return `${valor("year")}-${valor("month")}-${valor("day")}`;
}
function correspondePeriodoEstat(d) {
if (!dataInicioEstat && !dataFimEstat) return true;
const data = chaveDataInicioEstat(d);
if (!data) return false;
return (!dataInicioEstat || data >= dataInicioEstat) && (!dataFimEstat || data <= dataFimEstat);
}
function alterarPeriodoEstatisticas() {
const inicio = document.getElementById("statsDateStart");
const fim = document.getElementById("statsDateEnd");
dataInicioEstat = inicio?.value || "";
dataFimEstat = fim?.value || "";
atalhoPeriodoEstat = inferirAtalhoPeriodoEstat();
salvarPeriodoEstatisticas();
periodoGraficoPadraoPendente = true;
sincronizarPeriodoEstatisticas();
renderEstatisticas();
// O botão "Período" do Dashboard usa este mesmo filtro. Ao alterar a data,
// atualizamos a série para que não permaneça visível uma resposta antiga.
if (typeof desenharGrafico === "function") desenharGrafico();
}
function limparPeriodoEstatisticas() {
dataInicioEstat = "";
dataFimEstat = "";
atalhoPeriodoEstat = "tudo";
salvarPeriodoEstatisticas();
periodoGraficoPadraoPendente = true;
sincronizarPeriodoEstatisticas();
renderEstatisticas();
if (typeof desenharGrafico === "function") desenharGrafico();
}
// Nomes de liga/torneio/modalidade: fonte de alta precisão. Só devolve uma
// modalidade quando o texto cita algo inequivocamente esportivo.
function normalizarEsporte(v) {
const s = String(v || "").trim().toLowerCase();
if (!s) return null;
// Nomes de clubes podem conter referências a outras modalidades, como
// "AFC Wimbledon" e "Le Mans FC". Esses sufixos são evidência de futebol e
// impedem que uma palavra isolada do nome altere a modalidade da partida.
const clubeDeFutebol = /\b(?:fc|afc|cf|sc|fk|ac|cd|ud|united|city|athletic|sporting|calcio)\b/.test(s);
if (/american\s+football|futebol\s+americano|\bnfl\b|\bncaaf\b|\bcfl\b|\bcfb\b|college\s+football|\bufl\b|\busfl\b|\bxfl\b/.test(s)) return "Futebol Americano";
if (!clubeDeFutebol && /tennis|t[eê]nis|\batp\b|\bwta\b|\bitf\b|grand\s+slam|australian\s+open|roland\s+garros|french\s+open|us\s+open|wimbledon|davis\s+cup|billie\s+jean\s+king/.test(s)) return "Tênis";
if (/basketball|basquete|\bnba\b|\bwnba\b|\bncaab\b|euroleague|eurocup|\bacb\b|liga\s+endesa|\bnbl\b|\bcba\b|b\.league|\bfiba\b/.test(s)) return "Basquete";
if (/baseball|beisebol|\bmlb\b|\bnpb\b|\bkbo\b|\bcpbl\b|\bmilb\b|world\s+baseball\s+classic/.test(s)) return "Beisebol";
if (/hockey|h[oó]quei|\bnhl\b|\bahl\b|\bkhl\b|\bshl\b|\bliiga\b|ice\s+hockey/.test(s)) return "Hóquei";
if (/\bufc\b|\bmma\b|bellator|\bpfl\b|mixed\s+martial|boxing|\bboxe\b|heavyweight\s+title|one\s+championship/.test(s)) return "MMA & Boxe";
if (/\bgolf\b|\bgolfe\b|\bpga\b|liv\s+golf|ryder\s+cup|the\s+masters\b|open\s+championship|dp\s+world\s+tour/.test(s)) return "Golfe";
if (!clubeDeFutebol && /formula\s*1|\bf1\b|nascar|motogp|indycar|grand\s+prix|\bwrc\b|le\s+mans/.test(s)) return "Automobilismo";
if (/\be[-\s]?sports?\b|league\s+of\s+legends|lol\s+worlds|counter[-\s]?strike|\bcs2\b|cs:?go|\bdota\b|valorant|overwatch|rocket\s+league/.test(s)) return "Esports";
if (/\bcricket\b|cr[ií]quete|\bipl\b|\bt20\b|\bbbl\b|test\s+match/.test(s)) return "Críquete";
if (/\brugby\b|six\s+nations|\bnrl\b|super\s+rugby/.test(s)) return "Rugby";
if (/volleyball|v[oô]lei|\bfivb\b/.test(s)) return "Vôlei";
if (/soccer|futebol|\bepl\b|premier\s+league|la\s+liga|laliga|serie\s+a|serie\s+b|bundesliga|ligue\s*1|ligue\s*2|eredivisie|champions\s+league|europa\s+league|conference\s+league|libertadores|sul[-\s]?americana|copa\s+do\s+brasil|\bmls\b|\bnwsl\b|brasileirao|brasileiro|saudi\s+(?:pro|club)|\bspl\b|super\s+lig|liga\s+mx|liga\s+portugal|primeira\s+liga|championship|fa\s+cup|coppa\s+italia|dfb\s+pokal|world\s+cup|copa\s+am[ée]rica|\beuro\s+20\d\d\b|\bfifa\b|\buefa\b|conmebol/.test(s)) return "Futebol";
return null;
}
// Temas não-esportivos, do mais específico para o mais genérico.
const REGRAS_TEMA = [
["Cripto", /\b(bitcoin|btc|ethereum|ether|eth|solana|dogecoin|doge|xrp|ripple|cardano|litecoin|chainlink|crypto|cripto|criptomoeda|altcoin|stablecoin|usdc|usdt|memecoin|nfts?|binance|coinbase|blockchain|satoshi|halving|defi|hyperliquid|onchain)\b/],
["Geopolítica", /\b(war|guerra|ceasefire|cessar[-\s]?fogo|armistice|invasion|invas[ãa]o|nuclear\s+(?:weapon|test|strike)|missile|m[ií]ssil|airstrike|air\s+strike|troops|tropas|nato|otan|hostages?|ref[ée]ns?|peace\s+(?:deal|talks|plan|agreement)|acordo\s+de\s+paz|annex|sanctions?|san[çc][õo]es|coup\b|golpe\s+de\s+estado|drone\s+strike|blockade)\b/],
["Política", /\b(election|elei[çc][ãa]o|elei[çc][õo]es|eleitoral|president|presidente|presidential|primary\s+(?:election|winner)|caucus|senate|senado|senador|house\s+of\s+representatives|congress|congresso|governor|governador|prime\s+minister|primeiro[-\s]?ministro|chancellor|impeachment|impeached|cabinet|nominee|nomination|ballot|electoral\s+college|swing\s+state|democrats?|republicans?|\bgop\b|trump|biden|harris|vance|newsom|obama|lula|bolsonaro|milei|maduro|macron|starmer|orb[áa]n|netanyahu|zelensky|putin|xi\s+jinping|kim\s+jong|supreme\s+court|\bstf\b|\btse\b|mayor\b|prefeito|referendum|plebiscito|parliament|parlamento|shutdown|executive\s+order|pardon|indicted?|deportations?|epstein|nobel\s+peace)\b/],
["Economia", /\b(fed\b|federal\s+reserve|fomc|powell|interest\s+rates?|rate\s+(?:cut|hike)|juros|selic|\bcpi\b|inflation|infla[çc][ãa]o|\bgdp\b|\bpib\b|recession|recess[ãa]o|unemployment|desemprego|jobs\s+report|payrolls|treasury|yields?|s&p\s*500|nasdaq|dow\s+jones|ibovespa|stock\s+price|share\s+price|market\s+cap|earnings|\bipo\b|acquisition|merger|bankrupt|d[óo]lar|exchange\s+rate|oil\s+price|gold\s+price|debt\s+ceiling|tariffs?)\b/],
["Tecnologia & IA", /\b(openai|anthropic|claude|chatgpt|gpt[-\s]?\d|\bgpt\b|gemini|llama\s*\d|grok|deepseek|mistral|midjourney|sora|\bagi\b|artificial\s+intelligence|intelig[êe]ncia\s+artificial|\bllms?\b|\bai\s+(?:model|lab|agent|safety|race|company)\b|arc[-\s]agi|nvidia|apple\b|iphone|android|tesla|robotaxi|self[-\s]driving|waymo|neuralink|quantum\s+comput|tiktok|twitter|meta\s+ai|google\b|microsoft|amazon\b|semiconductor)\b/],
["Esports", /\b(league\s+of\s+legends|counter[-\s]?strike|\bcs2\b|cs:?go|\bdota\b|valorant|overwatch|e[-\s]?sports?)\b/],
["Games", /\b(gta\b|grand\s+theft\s+auto|rockstar\s+games|steam\b|nintendo|switch\s*2|playstation|\bps5\b|xbox|elden\s+ring|zelda|pok[ée]mon|minecraft|fortnite|roblox|call\s+of\s+duty|game\s+awards|videogames?|video\s+games?|speedrun|half[-\s]life|silksong|hollow\s+knight)\b/],
["Entretenimento", /\b(oscars?\b|academy\s+award|grammys?\b|emmys?\b|golden\s+globe|box\s+office|bilheteria|movie|filmes?\b|netflix|\bhbo\b|disney|spotify|billboard|albums?\b|[áa]lbum|taylor\s+swift|kanye|beyonc[ée]|rotten\s+tomatoes|\bimdb\b|halftime\s+show|miss\s+universe|big\s+brother|\bbbb\b|reality\s+show|celebrity|celebridade|div[óo]rcio|divorce|engaged|noivado|met\s+gala|person\s+of\s+the\s+year|youtube|mrbeast|podcast)\b/],
["Ciência & Saúde", /\b(nasa\b|spacex|starship|falcon\s+9|rocket\s+launch|lan[çc]amento\s+espacial|asteroid|\bmars\b|moon\s+landing|eclipse|nobel\s+prize|vaccines?|vacina|covid|h5n1|bird\s+flu|gripe\s+avi[áa]ria|outbreak|surto|\bfda\b|clinical\s+trial|cancer|c[âa]ncer|cure\s+for|measles|pandemic|pandemia|\bcern\b|nuclear\s+fusion|fus[ãa]o\s+nuclear|earthquake|terremoto|volcano|vulc[ãa]o|alien|extraterrestre)\b/],
["Clima", /\b(temperature|temperatura|hurricanes?|furac[ãa]o|tornado|snowfall|nevasca|rainfall|precipita[çc][ãa]o|heat\s+wave|onda\s+de\s+calor|el\s+ni[nñ]o|la\s+ni[nñ]a|weather|clima\b|wildfire|inc[êe]ndio\s+florestal|drought|seca\b|\bco2\b|global\s+warming|aquecimento\s+global|hottest\s+(?:year|month|day))\b/]
];
function categoriaPorTema(texto) {
for (const [nome, re] of REGRAS_TEMA) {
if (re.test(texto)) return nome;
}
return null;
}
// Reúne os textos da aposta separando metadado oficial (confiável) de
// título/slug (heurístico).
function sinaisDaAposta(d) {
const eventMeta = d?._sportMeta;
// Caches antigos misturavam tags do evento com todas as perguntas e nomes
// dos mercados irmãos (por exemplo, o filme "F1" dentro do evento do Oscar).
const marketTexts = new Set((eventMeta?.markets || []).flatMap(m =>
[m.slug, m.question, m.groupItemTitle, m.sportsMarketType]).filter(Boolean).map(v => String(v).toLowerCase()));
const metaTexts = Array.isArray(eventMeta?.eventTexts) ? eventMeta.eventTexts :
(Array.isArray(eventMeta?.texts) ? eventMeta.texts.filter(v => !marketTexts.has(String(v).toLowerCase())) : []);
const modalidadeContrato = String(d?._platform||'').toLowerCase()==='kalshi'
? globalThis.BettyMarketTitles.sportFromKalshiRules(d?._kalshiTitleMeta?.market) : null;
const meta = [d?.sport, d?.sportName, d?.category, d?.sportsCategory, d?.sportsMarketType, modalidadeContrato, ...metaTexts]
.filter(v => v != null && String(v).trim() !== "").map(v => String(v));
const slugs = [d?.slug, d?.eventSlug, d?._eventTicker, d?.seriesTicker]
.filter(v => v != null && String(v).trim() !== "").map(v => String(v));
const title = String(d?.title || "");
const limpar = s => s.toLowerCase().replace(/[_\-]+/g, " ").replace(/\s+/g, " ").trim();
return {
meta,
metaTexto: limpar(meta.join(" ")),
slugs,
title,
texto: limpar(title + " " + slugs.join(" ") + " " + meta.join(" "))
};
}
// Tickers de série são a evidência mais forte de modalidade nos dois sites.
function esportePorTicker(slugs) {
for (const raw of slugs) {
const t = String(raw).toUpperCase();
if (/^KX(?:NFL|NCAAF|CFL|UFL|USFL|XFL)/.test(t)) return "Futebol Americano";
if (/^KX(?:MLB|NPB|KBO|CPBL|MILB|WBC)/.test(t)) return "Beisebol";
if (/^KX(?:NBA|WNBA|NCAAB|EUROLEAGUE|ACB|NBL|CBA|BSL)/.test(t)) return "Basquete";
// Séries internacionais e ligas de basquete da Kalshi. O payload delas vem
// apenas com a categoria genérica "Sports", então o ticker é a fonte estável.
if (/^KX(?:FIBA|DBB|BBINTL|SOWBBALL|VBA|LNBP|SESBL|JBLEAGUE|LBP)/.test(t)) return "Basquete";
// Séries sem modalidade no payload: Copa del Rey, Taça de Portugal e liga indonésia.
// Não inferir futebol só por "goals": hóquei também usa esse termo.
if (/^KX(?:COPADELREY|TACAPORT|IDNSL)/.test(t)) return "Futebol";
if (/^KX(?:NHL|AHL|KHL|SHL|LIIGA|DEL)/.test(t)) return "Hóquei";
if (/^KX(?:ATP|WTA|ITF|TENNIS)/.test(t)) return "Tênis";
if (/^KX(?:UFC|MMA|BOX)/.test(t)) return "MMA & Boxe";
if (/^KX(?:PGA|LIV|GOLF|RYDER)/.test(t)) return "Golfe";
if (/^KX(?:F1|NASCAR|MOTOGP|INDY)/.test(t)) return "Automobilismo";
if (/^KX(?:LOL|CSGO|CS2|DOTA|VALORANT|ESPORT)/.test(t)) return "Esports";
if (/^KX(?:CRICKET|IPL)/.test(t)) return "Críquete";
if (/^KX(?:RUGBY|NRL|SIXNATIONS)/.test(t)) return "Rugby";
if (/^(?:KX)?(?:AFCON|CLUBF|EPL|BUNDES|LIGUE|EREDIV|SERIEA|SERIEB|SERIEC|PERLIGA|ASEAN|UEL|UECL|UCLW?|LIBERTADOR|SULAMERICAN|COPADOBRASIL|BRASILEIRO|SAUDI|SPL|LALIGA|MLS|NWSL|CHNSL|WORLDCUP|U20WWC|ARGNACB|DIMAYOR|BOLPDIV|LEAGUESCUP|GRECUP|SRBSL|DFBPOKAL|COPPAITALIA|ETTAN|SCOCUP|SVKCUP|USL|J[123]?LEAGUE|EFLCUP|LIGAEXP|EL1|FL1)/.test(t)) return "Futebol";
// Nos slugs esportivos da Polymarket, o prefixo `bk` identifica a liga de
// basquete (bkarg, bkbsl, bkeurocup, bkfiba...). Só é usado como fallback
// quando o Gamma não disponibiliza o metadado oficial do evento.
if (/^BK[A-Z0-9]+-/.test(t)) return "Basquete";
}
return null;
}
// Tipos de mercado da Gamma são campos estruturados e não dependem do nome
// do torneio, do time ou de abreviações dentro do slug. Usamos apenas os
// prefixos que identificam uma modalidade de forma inequívoca.
function esportePorTiposMercado(d) {
const ownMarket = globalThis.BettyMarketTitles?.exactMarket(d);
const tipos = [d?.sportsMarketType, ...(ownMarket ? [ownMarket.sportsMarketType] : (d?._sportMeta?.sportsMarketTypes || []))]
.map(v => String(v || "").toLowerCase()).filter(Boolean);
if (tipos.some(tipo => /^tennis_/.test(tipo))) return "Tênis";
if (tipos.some(tipo => /^basketball_/.test(tipo))) return "Basquete";
if (tipos.some(tipo => /^baseball_/.test(tipo))) return "Beisebol";
if (tipos.some(tipo => /^hockey_/.test(tipo))) return "Hóquei";
if (tipos.some(tipo => /^soccer_/.test(tipo))) return "Futebol";
if (tipos.some(tipo => /^(?:football|nfl|ncaaf)_/.test(tipo))) return "Futebol Americano";
if (tipos.some(tipo => /^(?:ufc|mma|boxing)_/.test(tipo))) return "MMA & Boxe";
if (tipos.some(tipo => /^(?:cs2|lol|dota|valorant)_/.test(tipo))) return "Esports";
return null;
}
// Slugs de confronto: "epl-ars-che-2026-01-12-..." (Polymarket) e
// "KXNBAGAME-..." (Kalshi). A forma já diz que é uma partida.
function slugDeConfronto(slugs) {
for (const raw of slugs) {
const s = String(raw).toLowerCase();
if (/^[a-z]{2,6}-[a-z0-9]{2,5}-[a-z0-9]{2,5}-\d{4}-\d{2}-\d{2}/.test(s)) return true;
if (/^kx[a-z]*(?:game|match|series)-/.test(s)) return true;
}
return false;
}
// Metadado oficial dizendo "isto é esporte" (categoria/tag do evento).
function metadadoDizEsporte(metaTexto) {
return /\b(sports?|esportes?|soccer|football|basketball|baseball|hockey|tennis|golf|boxing|cricket|rugby|olympics?|olimp[ií]adas?)\b/.test(metaTexto);
}
// Heurística fraca: só entra quando o título tem cara de confronto.
function esportePorConfronto(texto) {
if (!/\bvs\.?\b|\bversus\b|\s@\s/.test(texto)) return null;
if (/\b(quarter|touchdown|field goal|passing yards|rushing yards|receiving yards|first down|sack|interception)\b/.test(texto)) return "Futebol Americano";
if (/\b(home run|strikeout|innings?|pitcher|runs? scored|bases?|rbis?)\b/.test(texto)) return "Beisebol";
if (/\b(rebounds?|assists?|three[ -]?pointers?|quarters?|points?)\b/.test(texto)) return "Basquete";
if (/\b(goals?|gols?|both teams to score|draw no bet|corners?|cards?|clean sheet|first half|second half)\b/.test(texto)) return "Futebol";
// Sufixos de clube (FC, SC, United, Sporting...) são típicos de futebol.
if (/\b(fc|fk|cf|sc|ac|afc|sk|bk|cd|united|utd|city|athletic|atl[ée]tico|dynamo|dinamo|sporting|inter|club|calcio|hotspur|rovers|albion)\b/.test(texto)) return "Futebol";
if (/\b(sets?|aces?|double faults?|tie ?break|open|masters|challenger)\b/.test(texto)) return "Tênis";
if (/\b(rounds?|knockout|\bko\b|submission|by decision)\b/.test(texto)) return "MMA & Boxe";
if (/\b(moneyline|spread|o\/u|over\/under)\b/.test(texto)) return ESPORTE_GENERICO;
return null;
}
const _cacheCategoria = new WeakMap();
function temaDoMercadoProprio(d) {
const ownMarket = globalThis.BettyMarketTitles?.exactMarket(d);
const texto = [d.title, ownMarket?.question, d.slug, ownMarket?.slug]
.filter(Boolean).join(' ').toLowerCase().replace(/[_-]+/g, ' ');
// Especiais da transmissão e da cerimônia são entretenimento, mesmo quando
// o evento pai traz NFL/Sports. A decisão usa somente o contrato apostado.
if (/\b(?:halftime\s+show|national\s+anthem|coin\s+toss|gatorade\s+shower|streaker|jersey\s+number)\b/.test(texto) ||
/\b(?:run|air)\s+an?\s+ad\b|\b(?:commercial|advertisement)s?\b/.test(texto) ||
/\b(?:perform|sing)\b.*(?:["“]|\bsongs?\b)|\b(?:first|last)\s+song\b/.test(texto) ||
/\b(?:super\s+bowl|big\s+game)\b/.test(texto) && /\b(?:viewers?|watch|audience|ratings?|attend|proposes?|be\s+said|mentions?)\b/.test(texto) ||
/\bcry\b.*\bworld\s+cup\b|\bneymar\s+play\s+in\s+the\s+world\s+cup\b/.test(texto)) return 'Entretenimento';
// Nomes de atletas/equipes não são temas: Lloyd Harris não é Kamala Harris,
// Claude Ishimwe não é uma IA, e Carolina Hurricanes não é previsão do clima.
return /\s(?:vs\.?|versus|@)\s/.test(texto) || slugDeConfronto([d.slug, d.eventSlug, d._eventTicker]) ? null : categoriaPorTema(texto);
}
// O assunto do contrato tem precedência sobre tags genéricas do evento pai.
function classificarAposta(d) {
if (!d || typeof d !== "object") return CATEGORIA_OUTROS;
if (d._platform === "predict" || d._platform === "sxbet") return d._category || CATEGORIA_OUTROS;
const cache = _cacheCategoria.get(d);
if (cache) return cache;
const s = sinaisDaAposta(d);
// Tipos estruturados da Gamma vêm antes de textos livres. Assim, um torneio
// de tênis com "Championships" não é confundido com futebol.
let cat = temaDoMercadoProprio(d) || esportePorTiposMercado(d);
if (!cat) for (const v of s.meta) {
cat = normalizarEsporte(v);
if (cat) break;
}
if (!cat) cat = esportePorTicker(s.slugs);
if (!cat && (metadadoDizEsporte(s.metaTexto) || slugDeConfronto(s.slugs))) {
cat = normalizarEsporte(s.texto) || esportePorConfronto(s.texto) || ESPORTE_GENERICO;
}
if (!cat) cat = normalizarEsporte(s.texto);
if (!cat) cat = categoriaPorTema(s.texto);
if (!cat) cat = esportePorConfronto(s.texto);
if (!cat) cat = CATEGORIA_OUTROS;
_cacheCategoria.set(d, cat);
return cat;
}
function atualizarFiltroEsporteUI() {
const el = document.getElementById("statsSportFilter");
if (!el) return;
const fromDb = resumoEstatDbAtual()?.categorias;
const categorias = Array.isArray(fromDb) && fromDb.length ? fromDb : [...new Set(dadosEstat().map(classificarAposta))];
const ordered = ORDEM_CATEGORIA.filter(x => categorias.includes(x)).concat(categorias.filter(c => !ORDEM_CATEGORIA.includes(c)));
if (filtroEsporteEstat !== "todos" && !ordered.includes(filtroEsporteEstat)) ordered.push(filtroEsporteEstat);
el.innerHTML = '<option value="todos">Todas</option>' + ordered.map(e => `<option value="${escapeHtml(e)}">${escapeHtml(e)}</option>`).join("");
el.value = ordered.includes(filtroEsporteEstat) ? filtroEsporteEstat : "todos";
}
function alterarFiltroEsporte(valor) {
filtroEsporteEstat = valor || "todos";
salvarPeriodoEstatisticas();
renderEstatisticas();
}
// FONTE ÚNICA DAS ESTATÍSTICAS:
// exatamente o mesmo array de fechadas que alimenta a tabela de Posições.
// Não usamos números pré-calculados, cache de estatísticas ou outra fonte.
function apostasFechadasNoSite() {
return filtrarExcluidos(Array.isArray(fechados) ? fechados : []).filter(apostaNoPeriodoFechadasConfigurado);
}
function apostaNoPeriodoFechadasConfigurado(aposta) {
const { inicio, fim } = periodoFechadasDaCasa(aposta);
if (!inicio && !fim) return true;
const ymd = ymdBrasil(dataPosicaoTabela(aposta, false));
if (!ymd) return false;
return (!inicio || ymd >= inicio) && (!fim || ymd <= fim);
}
function listaEstatFiltrada() {
const base = apostasFechadasNoSite();
return base.filter(d =>
(filtroEsporteEstat === "todos" || classificarAposta(d) === filtroEsporteEstat) &&
(filtroCasaEstat === "todas" || casaDaAposta(d) === filtroCasaEstat) &&
correspondePeriodoEstat(d) && correspondeOddsEstat(d)
);
}
// Base de análise: posições fechadas (é onde o resultado é conhecido de fato)
function dadosEstat() {
return listaEstatFiltrada();
}
let resumoEstatDb = null;
let resumoEstatDbChave = "";
let resumoEstatDbEmAndamento = null;
function chaveResumoEstatDb() {
return [filtroCasaEstat || "todas", filtroEsporteEstat || "todos", filtroOddsEstat || "todas", dataInicioEstat || "", dataFimEstat || "", chavePeriodoFechadasConfigurado()].join("|");
}
function metricasResumoDb(raw) {
const dados = raw || {};
const profitFactor = dados.profitFactor == null && Number(dados.ganhoBruto) > 0 ? Infinity : Number(dados.profitFactor);
return {
total: Number(dados.total) || 0,
lucroD: Number(dados.lucroD) || 0, volumeD: Number(dados.volumeD) || 0,
lucro: unidadeEstat === "stake" ? (Number(dados.lucroStake) || 0) : (Number(dados.lucroD) || 0),
volume: unidadeEstat === "stake" ? (Number(dados.volumeStake) || 0) : (Number(dados.volumeD) || 0),
roi: Number(dados.roi) || 0, wins: Number(dados.wins) || 0, losses: Number(dados.losses) || 0,
taxaAcerto: Number(dados.taxaAcerto) || 0, oddMedia: Number(dados.oddMedia) || 0,
avgPnl: unidadeEstat === "stake" ? (Number(dados.avgPnlStake) || 0) : (Number(dados.avgPnl) || 0),
avgWin: Number(dados.avgWin) || 0, avgLoss: Number(dados.avgLoss) || 0,
profitFactor, ganhoBruto: Number(dados.ganhoBruto) || 0, perdaBruta: Number(dados.perdaBruta) || 0,
ganhoStake: Number(dados.ganhoStake) || 0, perdaStake: Number(dados.perdaStake) || 0
};
}
function resumoEstatDbAtual() {
return indiceDisponivel() && resumoEstatDb && resumoEstatDbChave === chaveResumoEstatDb() ? resumoEstatDb : null;
}
async function carregarResumoEstatDb(opcoes = {}) {
if (!indiceHistoricoLegivel()) return null;
const chave = chaveResumoEstatDb();
const incluirMercados = opcoes.incluirMercados || subAbaEstat === "mercado";
const incluirResumo = subAbaEstat === "resumo";
if (resumoEstatDb && resumoEstatDbChave === chave && (!incluirMercados || Array.isArray(resumoEstatDb.porMercado)) && (!incluirResumo || Array.isArray(resumoEstatDb.segmentos))) return resumoEstatDb;
const versao = versaoLeituraHistorico();
if (resumoEstatDbEmAndamento?.chave === chave && resumoEstatDbEmAndamento.versao === versao) {
const pending = resumoEstatDbEmAndamento;
const data = await pending.promise;
// A click on Markets while the base summary is loading must upgrade it once,
// not start duplicate queries or publish a response for an obsolete filter.
if (incluirMercados && !pending.incluirMercados && subAbaEstat === "mercado" && chaveResumoEstatDb() === chave && resumoHistoricoPodeSerPublicado(versao)) return carregarResumoEstatDb();
if (incluirResumo && !pending.incluirResumo && subAbaEstat === "resumo" && chaveResumoEstatDb() === chave && resumoHistoricoPodeSerPublicado(versao)) return carregarResumoEstatDb();
return data;
}
const params = new URLSearchParams({
category: filtroEsporteEstat || "todos",
platform: filtroCasaEstat || "todas",
oddsRange: filtroOddsEstat || "todas",
...(incluirMercados ? { includeMarkets: "1" } : {}),
...(incluirResumo ? { includeSegments: "1" } : {}),
start: dataInicioEstat || "",
end: dataFimEstat || ""
});
const promise = fetch(urlLeituraHistorico('/api/history/closed/stats?' + params.toString()), { cache: 'no-store' })
.then(r => { if (!r.ok) throw new Error('estatísticas HTTP ' + r.status); return r.json(); })
.then(data => {
if (!data?.ok) throw new Error('estatísticas inválidas');
if (!resumoHistoricoPodeSerPublicado(versao)) return null;
if (chaveResumoEstatDb() === chave) {
resumoEstatDb = { ...data,
...(resumoEstatDbChave === chave && Array.isArray(resumoEstatDb?.porMercado) && !Array.isArray(data.porMercado) ? {porMercado:resumoEstatDb.porMercado} : {}),
...(resumoEstatDbChave === chave && Array.isArray(resumoEstatDb?.segmentos) && !Array.isArray(data.segmentos) ? {segmentos:resumoEstatDb.segmentos} : {}) };
resumoEstatDbChave = chave;
}
return data;
}).catch(e => { console.warn('Estatísticas no SQLite indisponíveis:', e); return null; })
.finally(() => { if (resumoEstatDbEmAndamento?.chave === chave && resumoEstatDbEmAndamento?.versao === versao) resumoEstatDbEmAndamento = null; });
resumoEstatDbEmAndamento = { chave, versao, incluirMercados, incluirResumo, promise };
return promise;
}
function computeStatsGerais(lista) {
const total = lista.length;
const lucroD = lista.reduce((s, d) => s + (d.realizedPnl || 0), 0);
const volumeD = lista.reduce((s, d) => s + getStake(d), 0);
const roi = volumeD > 0 ? (lucroD / volumeD) * 100 : 0;
const pnls = lista.map(d => Number(d.realizedPnl) || 0);
const wins = pnls.filter(v => v > 0).length;
const losses = pnls.filter(v => v < 0).length;
const taxaAcerto = total ? (wins / total) * 100 : 0;
const odds = lista.map(d => getOddNum(d)).filter(o => o > 0);
const oddMedia = odds.length ? odds.reduce((s, o) => s + o, 0) / odds.length : 0;
const ganhoBruto = pnls.filter(v => v > 0).reduce((s, v) => s + v, 0);
const perdaBruta = pnls.filter(v => v < 0).reduce((s, v) => s + v, 0);
const avgWin = wins ? ganhoBruto / wins : 0;
const avgLoss = losses ? perdaBruta / losses : 0;
const profitFactor = Math.abs(perdaBruta) > 0 ? ganhoBruto / Math.abs(perdaBruta) : (ganhoBruto > 0 ? Infinity : 0);
let lucro = lucroD, volume = volumeD, avgPnl = total ? lucroD / total : 0;
if (typeof unidadeEstat !== "undefined" && unidadeEstat === "stake") {
lucro = lista.reduce((s, d) => s + dolarParaUnidades(d.realizedPnl || 0, getTsPnL(d), d._platform), 0);
volume = lista.reduce((s, d) => s + dolarParaUnidades(getStake(d), getTsPnL(d), d._platform), 0);
avgPnl = total ? lucro / total : 0;
}
return { total, lucro, volume, roi, wins, losses, taxaAcerto, oddMedia, lucroD, volumeD, avgPnl, avgWin, avgLoss, profitFactor, ganhoBruto, perdaBruta };
}
function computeMetricasAvancadas(lista) {
const pnls = [...lista].sort((a, b) => getTsResultadoPnL(a) - getTsResultadoPnL(b)).map(d => ({
pnl: unidadeEstat === "stake" ? dolarParaUnidades(d.realizedPnl || 0, getTsPnL(d), d._platform) : (Number(d.realizedPnl) || 0),
ts: getTsResultadoPnL(d)
}));
const vals = pnls.map(p => p.pnl);
const n = vals.length;
const sorted = [...vals].sort((a, b) => a - b);
const medianPnl = n ? (n % 2 ? sorted[(n - 1) / 2] : (sorted[n / 2 - 1] + sorted[n / 2]) / 2) : 0;
let maxWin = 0, maxLoss = 0, curWin = 0, curLoss = 0;
for (const v of vals) {
if (v > 0) { curWin++; curLoss = 0; if (curWin > maxWin) maxWin = curWin; }
else if (v < 0) { curLoss++; curWin = 0; if (curLoss > maxLoss) maxLoss = curLoss; }
else { curWin = 0; curLoss = 0; }
}
let equity = 0, hwm = 0, maxDD = 0, troughTs = 0, peakBefore = 0, recoveryTs = null;
const resultadoPorDia = new Map();
for (const p of pnls) {
if (!(p.ts > 0)) continue;
const dia = ymdBrasil(new Date(p.ts));
const bucket = resultadoPorDia.get(dia) || { pnl: 0, ts: 0 };
bucket.pnl += p.pnl; bucket.ts = Math.max(bucket.ts, p.ts);
resultadoPorDia.set(dia, bucket);
}
for (const p of resultadoPorDia.values()) {
equity += p.pnl;
if (equity >= hwm) {
if (maxDD < 0 && recoveryTs == null && troughTs && equity >= peakBefore) recoveryTs = p.ts;
hwm = equity;
} else {
const dd = equity - hwm;
if (dd < maxDD) { maxDD = dd; troughTs = p.ts; peakBefore = hwm; recoveryTs = null; }
}
}
const recoveryDays = recoveryTs && troughTs ? (recoveryTs - troughTs) / 86400000 : null;
const byWeek = new Map(), byMonth = new Map();
for (const d of lista) {
const tsResultado = getTsResultadoPnL(d);
const ymd = tsResultado ? ymdBrasil(new Date(tsResultado)) : '';
if (!ymd) continue;
const pnl = unidadeEstat === "stake" ? dolarParaUnidades(d.realizedPnl || 0, getTsPnL(d), d._platform) : (Number(d.realizedPnl) || 0);
const month = ymd.slice(0, 7);
byMonth.set(month, (byMonth.get(month) || 0) + pnl);
const [y, m, dd] = ymd.split("-").map(Number);
const date = new Date(Date.UTC(y, m - 1, dd));
const day = date.getUTCDay() || 7;
date.setUTCDate(date.getUTCDate() + 4 - day);
const year = date.getUTCFullYear();
const yearStart = new Date(Date.UTC(year, 0, 1));
const week = Math.ceil((((date - yearStart) / 86400000) + 1) / 7);
const wk = `${year}-W${String(week).padStart(2, "0")}`;
byWeek.set(wk, (byWeek.get(wk) || 0) + pnl);
}
return {
medianPnl, maxWinStreak: maxWin, maxLossStreak: maxLoss, maxDrawdown: maxDD,
currentDrawdown: equity - hwm, recoveryDays: recoveryDays == null ? null : Math.round(recoveryDays * 10) / 10,
recovered: recoveryTs != null,
pnlSemana: [...byWeek.entries()].sort((a, b) => a[0].localeCompare(b[0])).map(([d, v]) => ({ d, v })),
pnlMes: [...byMonth.entries()].sort((a, b) => a[0].localeCompare(b[0])).map(([d, v]) => ({ d, v }))
};
}
function agruparPor(lista, chaveFn) {
const mapa = {};
lista.forEach(d => {
const k = chaveFn(d);
if (!mapa[k]) mapa[k] = [];
mapa[k].push(d);
});
return Object.entries(mapa).map(([chave, itens]) => ({ chave, ...computeStatsGerais(itens) }));
}
async function renderEstatisticas() {
// Preserva a última leitura confirmada enquanto o histórico é sincronizado.
if (!dadosFinanceirosConfirmados()) {
const statsEl = document.getElementById("statsEstat");
if (statsEl && (!statsEl.innerHTML.trim() || statsEl.textContent.includes("Carregando..."))) {
statsEl.innerHTML = [
[ICONS.trendUp, "Lucro Total"],
[ICONS.percent, "ROI"],
[ICONS.db, "Volume de Apostas"],
[ICONS.target, "Taxa de Acerto"],
[ICONS.trendUp, "Odd Média"]
].map(([icon,label]) => `<div class="stat-card"><div class="stat-icon blue">${icon}</div><div class="stat-body"><div class="label">${label}</div><div class="value">—</div><div class="sub">Sincronizando histórico...</div></div></div>`).join("");
}
const subContent = document.getElementById("statsSubContent");
if (subAbaEstat === "site" && subContent && !subContent.hasChildNodes()) subContent.innerHTML = renderPorSite(null, "Aguardando histórico");
return;
}
const versaoHistorico = versaoLeituraHistorico();
sincronizarPeriodoEstatisticas();
atualizarFiltroEsporteUI();
const chaveDb = chaveResumoEstatDb();
const resumoDb = await carregarResumoEstatDb();
if (!leituraHistoricoAindaAtual(versaoHistorico) || chaveDb !== chaveResumoEstatDb()) return;
if (!resumoDb) {
const statsEl = document.getElementById("statsEstat");
if (statsEl) statsEl.innerHTML = "";
const extra = document.getElementById("statsExtraMetrics");
if (extra) extra.innerHTML = "";
const subContent = document.getElementById("statsSubContent");
if (subContent) {
if (subAbaEstat === "site") subContent.innerHTML = renderPorSite();
else subContent.textContent = "Histórico indisponível";
}
return;
}
const g = metricasResumoDb(resumoDb.total);
const avancadas = resumoDb.avancadas;
atualizarFiltroEsporteUI();
renderFiltrosAtivosEstat();
const statsEl = document.getElementById("statsEstat");
if (statsEl) {
statsEl.innerHTML = `
<div class="stat-card">
<div class="stat-icon ${g.lucro >= 0 ? 'green' : 'red'}">${ICONS.trendUp}</div>
<div class="stat-body">
<div class="label">Lucro Total</div>
<div class="value ${g.lucro >= 0 ? 'positive' : 'negative'}">${fmtEstat(g.lucro, true)}</div>
</div>
</div>
<div class="stat-card">
<div class="stat-icon info">${ICONS.percent}</div>
<div class="stat-body">
<div class="label">ROI</div>
<div class="value ${g.roi >= 0 ? 'positive' : 'negative'}">${g.roi >= 0 ? '+' : ''}${g.roi.toFixed(2)}%</div>
</div>
</div>
<div class="stat-card">
<div class="stat-icon blue">${ICONS.db}</div>
<div class="stat-body">
<div class="label">Volume de Apostas</div>
<div class="value">${fmtEstat(g.volume, false)}</div>
<div class="sub">${g.total} apostas</div>
</div>
</div>
<div class="stat-card stat-card-centered-sub">
<div class="stat-icon purple">${ICONS.target}</div>
<div class="stat-body">
<div class="label">Taxa de Acerto</div>
<div class="value">${g.taxaAcerto.toFixed(1)}%</div>
<div class="sub">${g.wins} acertadas · ${g.losses || 0} perdas</div>
</div>
</div>
<div class="stat-card">
<div class="stat-icon info">${ICONS.trendUp}</div>
<div class="stat-body">
<div class="label">Odd Média</div>
<div class="value">${g.oddMedia > 0 ? g.oddMedia.toFixed(2) : '—'}</div>
</div>
</div>
`;
}
renderExtraMetricsEstat(g, avancadas);
renderSubAbaEstat();
}
function fmtProfitFactor(v) {
if (v == null || !Number.isFinite(Number(v))) return "∞";
return Number(v).toFixed(2);
}
function renderExtraMetricsEstat(g, avancadas) {
const el = document.getElementById("statsExtraMetrics");
if (!el) return;
const av = avancadas || {};
const median = unidadeEstat === "stake" ? (av.medianPnlStake ?? av.medianPnl) : av.medianPnl;
const tom = valor => Number(valor) > 0 ? "positive" : Number(valor) < 0 ? "negative" : "";
const items = [
["Expectativa / aposta", fmtEstat(g.avgPnl, true), tom(g.avgPnl)],
["Mediana / aposta", fmtEstat(median || 0, true), tom(median)],
["Ganho vs perda", `<span class="positive">${fmtEstat(g.avgWin, true)}</span> / <span class="negative">${fmtEstat(g.avgLoss, true)}</span>`, ""],
["Profit factor", fmtProfitFactor(g.profitFactor), Number(g.profitFactor) > 1 ? "positive" : Number(g.profitFactor) < 1 ? "negative" : ""],
["Sequências", `${av.maxWinStreak || 0} W · ${av.maxLossStreak || 0} L`, ""],
["Drawdown máx. diário", fmtEstat(av.maxDrawdown || 0, true), tom(av.maxDrawdown)]
];
el.innerHTML = items.map(([lbl, val, classe]) => `<div class="metric-mini"><div class="lbl">${lbl}</div><div class="val ${classe}">${val}</div></div>`).join("");
}
function trocarSubAbaEstat(sub) {
subAbaEstat = sub;
document.querySelectorAll(".stat-subtab").forEach(b => b.classList.toggle("active", b.dataset.sub === sub));
renderEstatisticas();
}
// Unidade nas Estatísticas: dólar ($) ou stake (u), conforme a casa da aposta.
let unidadeEstat = "dolar";
function fmtEstat(v, comSinal) {
const n = Number(v) || 0;
if (unidadeEstat === "stake") {
// v já vem em unidades (computeStatsGerais converte por data)
const abs = Math.abs(n);
const body = abs.toFixed(abs >= 100 ? 1 : 2) + " u";
if (comSinal) return (n > 0 ? "+" : n < 0 ? "-" : "") + body;
return (n < 0 ? "-" : "") + body;
}
if (comSinal) return (n >= 0 ? "+" : "") + fmtDinheiro(n);
return fmtDinheiro(n);
}
function alternarUnidadeEstat() {
unidadeEstat = unidadeEstat === "dolar" ? "stake" : "dolar";
const sw = document.getElementById("unitSwitch");
const lblStake = document.getElementById("unitLblStake");
const lblDolar = document.getElementById("unitLblDolar");
if (sw) sw.classList.toggle("on", unidadeEstat === "dolar");
if (lblStake) lblStake.classList.toggle("active", unidadeEstat === "stake");
if (lblDolar) lblDolar.classList.toggle("active", unidadeEstat === "dolar");
renderEstatisticas();
}
function renderSubAbaEstat() {
const el = document.getElementById("statsSubContent");
if (!el) return;
if (subAbaEstat === "site") el.innerHTML = renderPorSite();
else if (subAbaEstat === "mercado") el.innerHTML = renderPorMercado();
else if (subAbaEstat === "resumo") el.innerHTML = renderResumoGeral();
else el.innerHTML = renderPorEsporte();
}
function gerarBarraDivergente(itens) {
const maxAbs = Math.max(1, ...itens.map(i => Math.abs(i.ganho || 0)), ...itens.map(i => Math.abs(i.perda || 0)));
return `<div class="divbar-wrap">${itens.map(i => {
const ganho = Math.max(0, Number(i.ganho) || 0);
const perda = Math.abs(Math.min(0, Number(i.perda) || 0));
const wG = (ganho / maxAbs) * 100;
const wP = (perda / maxAbs) * 100;
const net = (Number(i.ganho) || 0) + (Number(i.perda) || 0);
return `<div class="divbar-row">
<div class="divbar-label" title="${escapeHtml(i.label)}">${escapeHtml(i.label)}</div>
${i.status ? `<div class="divbar-status">${escapeHtml(i.status)}</div>` : `<div class="divbar-track">
<div class="divbar-neg" style="width:${wP.toFixed(1)}%"></div>
<div class="divbar-pos" style="width:${wG.toFixed(1)}%"></div>
</div>
<div class="divbar-val" style="color:${net >= 0 ? 'var(--pos)' : 'var(--danger)'}">${fmtEstat(net, true)}</div>`}
</div>`;
}).join("")}</div>`;
}
function celulasMetricasExtras(row) {
return `<td style="color:${row.avgPnl >= 0 ? 'var(--pos)' : 'var(--danger)'}">${fmtEstat(row.avgPnl || 0, true)}</td>
<td>${fmtProfitFactor(row.profitFactor)}</td>`;
}
function gerarBarrasHorizontais(itens) {
const maxAbs = Math.max(1, ...itens.map(i => Math.abs(i.value)));
return itens.map((i, idx) => {
const pct = Math.max(2, (Math.abs(i.value) / maxAbs) * 100);
const cor = i.value >= 0 ? "#3ddc84" : "#ff6369";
return `
<div class="barh-row">
<div class="barh-label" title="${escapeHtml(i.label)}">${escapeHtml(i.label)}</div>
<div class="barh-track"><div class="barh-fill" style="width:0%;background:${cor};" data-w="${pct.toFixed(1)}"></div></div>
<div class="barh-val" style="color:${cor}">${fmtEstat(i.value, true)}</div>
</div>`;
}).join("");
}
function animarBarras(container) {
requestAnimationFrame(() => {
container.querySelectorAll(".barh-fill").forEach(el => {
el.style.width = el.dataset.w + "%";
});
});
}
function tabelaPnlPeriodo(avancadas) {
const weeks = (avancadas?.pnlSemana || []).slice(-10);
const months = (avancadas?.pnlMes || []).slice(-12);
if (!weeks.length && !months.length) return "";
const linhas = arr => arr.map(r => {
const valor = unidadeEstat === "stake" ? r.vStake : r.v;
return `<tr><td>${escapeHtml(r.d)}</td><td style="color:${valor >= 0 ? 'var(--pos)' : 'var(--danger)'}">${fmtEstat(valor, true)}</td></tr>`;
}).join("");
return `<div class="stat-grid-2 pnl-period-grid" style="margin-top:14px;">
<div class="card" style="padding:20px;">
<div class="stat-card-title">PnL por semana</div>
<div class="stat-card-sub">Últimas semanas do período filtrado</div>
<table class="pnl-period-table"><thead><tr><th>Semana ISO</th><th>PnL</th></tr></thead><tbody>${linhas(weeks) || '<tr><td colspan="2" style="color:var(--muted)">Sem dados</td></tr>'}</tbody></table>
</div>
<div class="card" style="padding:20px;">
<div class="stat-card-title">PnL por mês</div>
<div class="stat-card-sub">Últimos meses do período filtrado</div>
<table class="pnl-period-table"><thead><tr><th>Mês</th><th>PnL</th></tr></thead><tbody>${linhas(months) || '<tr><td colspan="2" style="color:var(--muted)">Sem dados</td></tr>'}</tbody></table>
</div>
</div>`;
}
function estadoSiteSemResultados(casa, estadoPendente) {
const estado = casaSync[casa]?.state;
if (estado === "error" || estado === "stale") return "Consulta à casa indisponível";
if (estado === "partial") return "Histórico parcial";
return estadoPendente;
}
function renderPorSite(resumoDb = resumoEstatDbAtual(), estadoPendente = "Histórico indisponível") {
const casas = filtroCasaEstat === "todas" ? CASAS_STAKE : [filtroCasaEstat];
const resultados = new Map((resumoDb?.porSite || []).map(row => [row.chave, metricasResumoDb(row)]));
const grupos = casas.filter(casa => BETTY_CONNECTIONS?.[casa] &&
(!resumoDb || Number(resultados.get(casa)?.total) > 0)).map(casa => ({
casa, label: NOMES_CASAS[casa], row: resultados.get(casa)
})).sort((a, b) => (b.row?.total ?? 0) - (a.row?.total ?? 0));
const g = resumoDb ? metricasResumoDb(resumoDb.total) : null;
const status = casa => estadoSiteSemResultados(casa, estadoPendente);
const vazio = resumoDb
? (filtroEsporteEstat === "todos" ? "Sem posições fechadas no recorte" : "Sem posições fechadas em " + filtroEsporteEstat)
: estadoPendente;
const linhas = grupos.map(({ casa, label, row }) => `
<tr>
<td><span class="site-cell"><img class="casa-logo" src="/assets/platforms/${casa}.png" alt="">${escapeHtml(label)}</span></td>
${row ? `<td>${row.total}</td>
<td>${fmtEstat(row.volume, false)}</td>
<td style="color:${row.lucro >= 0 ? 'var(--pos)' : 'var(--danger)'}">${fmtEstat(row.lucro, true)}</td>
<td style="color:${row.roi >= 0 ? 'var(--pos)' : 'var(--danger)'}">${row.roi >= 0 ? '+' : ''}${row.roi.toFixed(2)}%</td>
<td>${row.taxaAcerto.toFixed(1)}%</td>
<td>${row.oddMedia > 0 ? row.oddMedia.toFixed(2) : '—'}</td>
${celulasMetricasExtras(row)}` : `<td colspan="8" class="site-status">${escapeHtml(status(casa))}</td>`}
</tr>
`).join("");
const divItens = grupos.map(({ casa, label, row }) => row ? {
label, ganho: unidadeEstat === "stake" ? row.ganhoStake : row.ganhoBruto,
perda: unidadeEstat === "stake" ? row.perdaStake : row.perdaBruta
} : { label, status: status(casa) });
return `
<div class="stat-grid-2 site-performance-grid">
<div class="card" style="padding:20px;">
<div class="stat-card-title">Desempenho por Site</div>
<div class="stat-card-sub site-category-filter">
<span>Posições fechadas${filtroEsporteEstat === "todos" ? "" : " · " + escapeHtml(filtroEsporteEstat)}</span>
${filtroEsporteEstat === "todos" ? "" : '<button type="button" data-on-click="alterarFiltroEsporte(\'todos\')">Ver todas as categorias</button>'}
</div>
<div style="overflow-x:auto;">
<table class="stat-table site-table">
<thead><tr><th>Site</th><th>Apostas</th><th>Volume</th><th>Lucro</th><th>ROI</th><th>Taxa de Acerto</th><th>Odd Média</th><th>Expectativa</th><th>Profit factor</th></tr></thead>
<tbody>
${linhas || `<tr><td class="site-status" colspan="9">${escapeHtml(vazio)}</td></tr>`}
${g?.total ? `<tr class="total-row">
<td>Total</td><td>${g.total}</td><td>${fmtEstat(g.volume, false)}</td>
<td style="color:${g.lucro >= 0 ? 'var(--pos)' : 'var(--danger)'}">${fmtEstat(g.lucro, true)}</td>
<td style="color:${g.roi >= 0 ? 'var(--pos)' : 'var(--danger)'}">${g.roi >= 0 ? '+' : ''}${g.roi.toFixed(2)}%</td>
<td>${g.taxaAcerto.toFixed(1)}%</td><td>${g.oddMedia > 0 ? g.oddMedia.toFixed(2) : '—'}</td>
${celulasMetricasExtras(g)}
</tr>` : ""}
</tbody>
</table>
</div>
</div>
<div class="card" style="padding:20px;">
<div class="stat-card-title">Ganhos e perdas por Site</div>
<div class="stat-card-sub">Barra divergente — perdas à esquerda, ganhos à direita</div>
${gerarBarraDivergente(divItens)}
</div>
</div>
${tabelaPnlPeriodo(resumoDb?.avancadas)}
`;
}
function renderPorEsporte() {
const lista = dadosEstat();
const resumoDb = resumoEstatDbAtual();
// Toda aposta fechada aparece aqui: os totais desta visão batem com os
// cartões do topo. Esportes vêm primeiro (por modalidade), depois os demais
// temas; dentro de cada bloco, ordenamos pela quantidade de apostas.
const grupos = resumoDb ? (resumoDb.porCategoria || []).map(row => ({ chave: row.chave, ...metricasResumoDb(row) })) : agruparPor(lista, classificarAposta);
grupos.sort((a, b) => {
const ea = ehCategoriaEsporte(a.chave) ? 0 : 1;
const eb = ehCategoriaEsporte(b.chave) ? 0 : 1;
if (ea !== eb) return ea - eb;
if (a.chave === CATEGORIA_OUTROS) return 1;
if (b.chave === CATEGORIA_OUTROS) return -1;
return b.total - a.total || b.lucro - a.lucro;
});
const g = resumoDb ? metricasResumoDb(resumoDb.total) : computeStatsGerais(lista);
const linhas = grupos.map(row => `
<tr>
<td><span class="site-cell">${iconeCategoria(row.chave)}${escapeHtml(row.chave)}</span></td>
<td>${row.total}</td>
<td>${fmtEstat(row.volume, false)}</td>
<td style="color:${row.lucro >= 0 ? 'var(--pos)' : 'var(--danger)'}">${fmtEstat(row.lucro, true)}</td>
<td style="color:${row.roi >= 0 ? 'var(--pos)' : 'var(--danger)'}">${row.roi >= 0 ? '+' : ''}${row.roi.toFixed(2)}%</td>
<td>${row.taxaAcerto.toFixed(1)}%</td>
<td>${row.oddMedia > 0 ? row.oddMedia.toFixed(2) : '—'}</td>
${celulasMetricasExtras(row)}
</tr>
`).join("");
const barras = gerarBarrasHorizontais(grupos.map(row => ({ label: row.chave, value: row.lucro })));
setTimeout(() => {
const c = document.getElementById("statsSubContent");
if (c) animarBarras(c);
}, 0);
return `
<div class="stat-grid-2">
<div class="card" style="padding:20px;">
<div class="stat-card-title">Desempenho por Categoria</div>
<div class="stat-card-sub">Esportes por modalidade e os demais mercados por tema — classificação estimada a partir dos metadados, ticker e título</div>
<div style="overflow-x:auto;">
<table class="stat-table">
<thead><tr><th>Categoria</th><th>Apostas</th><th>Volume</th><th>Lucro</th><th>ROI</th><th>Taxa de Acerto</th><th>Odd Média</th><th>Expectativa</th><th>Profit factor</th></tr></thead>
<tbody>
${linhas || `<tr><td colspan="9" style="color:var(--muted);text-align:center;">Sem dados</td></tr>`}
<tr class="total-row">
<td>Total</td><td>${g.total}</td><td>${fmtEstat(g.volume, false)}</td>
<td style="color:${g.lucro >= 0 ? 'var(--pos)' : 'var(--danger)'}">${fmtEstat(g.lucro, true)}</td>
<td style="color:${g.roi >= 0 ? 'var(--pos)' : 'var(--danger)'}">${g.roi >= 0 ? '+' : ''}${g.roi.toFixed(2)}%</td>
<td>${g.taxaAcerto.toFixed(1)}%</td><td>${g.oddMedia > 0 ? g.oddMedia.toFixed(2) : '—'}</td>
${celulasMetricasExtras(g)}
</tr>
</tbody>
</table>
</div>
</div>
<div class="card" style="padding:20px;">
<div class="stat-card-title">Lucro por Categoria</div>
<div class="stat-card-sub">Valores negativos permanecem visíveis</div>
${barras || '<div style="color:var(--muted);font-size:0.85rem;">Sem dados</div>'}
</div>
</div>
`;
}
const mercadosMlExpandidos = new Set();
function iconeCategoria(category) {
// Actual WhatsApp artwork stored locally, independent of OS emoji fonts.
if (category === 'Tênis' || category === 'Futebol') {
const file=category === 'Tênis' ? 'whatsapp-tennis.png' : 'whatsapp-soccer-ball.png';
return `<span class="category-icon" aria-hidden="true"><img src="/assets/emoji/${file}" alt="" width="16" height="16" decoding="async"></span> `;
}
const emojis = {
'Futebol':'⚽', 'Futebol Americano':'🏈', 'Basquete':'🏀', 'Beisebol':'⚾', 'Tênis':'🎾', 'Hóquei':'🏒',
'MMA & Boxe':'🥊', 'Golfe':'⛳', 'Automobilismo':'🏎️', 'Esports':'🎮', 'Críquete':'🏏', 'Rugby':'🏉',
'Vôlei':'🏐', 'Outros Esportes':'🏅', 'Política':'🌎', 'Entretenimento':'🎬', 'Games':'🎮',
'Geopolítica':'🌐', 'Economia':'💰', 'Cripto':'🪙', 'Tecnologia & IA':'🤖', 'Ciência & Saúde':'🔬', 'Clima':'🌦️', 'Outros':'📌'
};
return emojis[category] ? `<span class="category-icon" aria-hidden="true">${emojis[category]}</span> ` : '';
}
function toggleMercadoMl(button) {
const category = button.dataset.category;
const group = button.dataset.group;
const key = category + '|' + group;
const expanded = !mercadosMlExpandidos.has(key);
if (expanded) mercadosMlExpandidos.add(key); else mercadosMlExpandidos.delete(key);
button.setAttribute('aria-expanded',String(expanded));
button.querySelector('.ml-arrow').textContent = expanded ? '▼' : '▶';
for (const row of button.closest('tbody').querySelectorAll('[data-market-detail]')) {
if (row.dataset.marketDetail === group) row.hidden = !expanded;
}
}
function renderPorMercado() {
const resumo = resumoEstatDbAtual();
if (resumo && !Array.isArray(resumo.porMercado)) {
return '<div class="card" style="padding:20px;">Reabra o aplicativo para carregar a análise por mercado nesta versão.</div>';
}
const groups = resumo ? resumo.porMercado.map(row => ({ chave: row.chave, ...metricasResumoDb(row) }))
  : agruparPor(dadosEstat(), d => BettyMarketBreakdown.key(tituloExibicao(d), classificarAposta(d)));
const categories = new Map();
for (const row of groups) {
const [category, market] = JSON.parse(row.chave);
if (!categories.has(category)) categories.set(category, []);
categories.get(category).push({ ...row, market });
}
const color = v => v > 0 ? 'var(--pos)' : v < 0 ? 'var(--danger)' : 'var(--text)';
const cells = r => `<td>${r.total}</td><td>${fmtEstat(r.volume,false)}</td><td style="color:${color(r.lucro)}">${fmtEstat(r.lucro,true)}</td><td style="color:${color(r.roi)}">${r.roi > 0 ? '+' : ''}${r.roi.toFixed(2)}%</td><td>${r.taxaAcerto.toFixed(1)}%</td><td>${r.oddMedia > 0 ? r.oddMedia.toFixed(2) : '—'}</td>${celulasMetricasExtras(r)}`;
const totals = resumo ? new Map(resumo.porCategoria.map(r => [r.chave, metricasResumoDb(r)]))
  : new Map(agruparPor(dadosEstat(),classificarAposta).map(r => [r.chave,r]));
const ordered = [...categories].sort((a,b) => (totals.get(b[0])?.total || 0) - (totals.get(a[0])?.total || 0));
return `<div class="card" style="padding:20px;">
<div class="stat-card-title">Desempenho por Mercado</div>
<div class="stat-card-sub">Posições fechadas por categoria, mercado, período e linha. Total do jogo e equipe/jogador ficam separados. Respeita os filtros acima.</div>
${ordered.map(([category,rows]) => {
rows.sort((a,b) => Number(Boolean(BettyMarketBreakdown.expandableGroup(category,b.market))) - Number(Boolean(BettyMarketBreakdown.expandableGroup(category,a.market))) || (ehCategoriaEsporte(category)
  ? BettyMarketBreakdown.compareFootball(a.market,b.market)
  : b.total-a.total || a.market.localeCompare(b.market,'pt-BR',{numeric:true})));
const total=totals.get(category);
const emitted = new Set();
const groupLabels = {ml:'ML — Todos',over:'Over gols — Todos',under:'Under gols — Todos',corners:'Escanteios — Todos',handicap:'Handicap — Todos',draw:'Empate — Todos',race:'Race — Todos',longterm:'Longo prazo',btts:'Ambas marcam — Todos'};
if (['MMA & Boxe','Golfe','Automobilismo','Esports','Críquete','Rugby','Vôlei','Outros Esportes'].includes(category)) {
groupLabels.over='Over — Todos';groupLabels.under='Under — Todos';
}
if (category === 'Futebol Americano' || category === 'Basquete') {
groupLabels.over = 'Over pontos — Todos';
groupLabels.under = 'Under pontos — Todos';
}
if (category === 'Beisebol') {
groupLabels.over = 'Over runs — Todos';
groupLabels.under = 'Under runs — Todos';
}
if (category === 'Tênis') {
groupLabels.over = 'Over games/sets — Todos';
groupLabels.under = 'Under games/sets — Todos';
}
const marketRows = rows.map(r => {
const group = BettyMarketBreakdown.expandableGroup(category,r.market);
const expanded = mercadosMlExpandidos.has(category + '|' + group);
let summary = '';
if (group && !emitted.has(group)) {
emitted.add(group);
const aggregate = resumo ? metricasResumoDb(BettyMarketBreakdown.aggregateMetrics(resumo.porMercado.filter(raw => {
const [cat,label] = JSON.parse(raw.chave); return cat === category && BettyMarketBreakdown.expandableGroup(cat,label) === group;
}))) : computeStatsGerais(dadosEstat().filter(d => classificarAposta(d) === category && BettyMarketBreakdown.expandableGroup(category,BettyMarketBreakdown.describe(tituloExibicao(d),category)) === group));
summary = `<tr><td><button type="button" data-category="${escapeHtml(category)}" data-group="${group}" aria-expanded="${expanded}" data-on-click="toggleMercadoMl(this)" style="background:transparent;border:0;color:inherit;font:inherit;cursor:pointer;padding:0;"><span class="ml-arrow">${expanded ? '▼' : '▶'}</span> ${groupLabels[group]}</button></td>${cells(aggregate)}</tr>`;
}
return summary + `<tr${group ? ` data-market-detail="${group}"${group === 'ml' ? ' data-ml-detail' : ''}${expanded ? '' : ' hidden'}` : ''}><td${group ? ' style="padding-left:30px;"' : ''}>${escapeHtml(r.market)}</td>${cells(r)}</tr>`;
}).join('');
return `<details open style="margin-top:18px;"><summary style="cursor:pointer;font-weight:700;padding:10px 0;">${iconeCategoria(category)}${escapeHtml(category)} · ${total?.total || 0} apostas · <span style="color:${color(total?.lucro || 0)}">${fmtEstat(total?.lucro || 0,true)}</span></summary>
<div style="overflow-x:auto;"><table class="stat-table"><thead><tr><th>Mercado</th><th>Apostas</th><th>Volume</th><th>Lucro</th><th>ROI</th><th>Taxa de Acerto</th><th>Odd Média</th><th>Expectativa</th><th>Profit factor</th></tr></thead>
<tbody>${marketRows}${total ? `<tr class="total-row"><td>Total ${escapeHtml(category)}</td>${cells(total)}</tr>` : ''}</tbody></table></div></details>`;
}).join('') || '<p style="color:var(--muted)">Sem apostas fechadas para os filtros selecionados.</p>'}
</div>`;
}
function calcularSequenciaAtual(lista) {
const ordenada = [...lista].sort((a, b) => getTsResultadoPnL(b) - getTsResultadoPnL(a));
if (!ordenada.length) return null;
const primeiraVitoria = (ordenada[0].realizedPnl || 0) > 0;
let n = 0;
for (const d of ordenada) {
const venceu = (d.realizedPnl || 0) > 0;
if (venceu !== primeiraVitoria) break;
n++;
}
return { venceu: primeiraVitoria, n };
}
function renderResumoGeral() {
const resumoDb = resumoEstatDbAtual();
if (!resumoDb || !Array.isArray(resumoDb.segmentos)) return '<p style="color:var(--muted);">Não foi possível carregar os segmentos. Tente abrir a aba novamente.</p>';
const g = metricasResumoDb(resumoDb.total);
if (!g.total) return '<div class="card" style="padding:20px;"><div class="stat-card-title">Resumo</div><p style="color:var(--muted);">Sem posições fechadas para os filtros selecionados.</p></div>';
const color = n => n > 0 ? 'var(--pos)' : n < 0 ? 'var(--danger)' : 'var(--text)';
const segmentos = resumoDb.segmentos.map(raw => {
const [category,market,odds] = JSON.parse(raw.chave);
return { ...metricasResumoDb(raw), category, market, odds };
});
const tie = (a,b) => b.total-a.total || a.category.localeCompare(b.category,'pt-BR') || a.market.localeCompare(b.market,'pt-BR') || a.odds.localeCompare(b.odds);
const lucros = segmentos.filter(r=>r.total>=50 && r.lucro>0).sort((a,b)=>b.lucro-a.lucro || tie(a,b)).slice(0,10);
const perdas = segmentos.filter(r=>r.total>=50 && r.lucro<0).sort((a,b)=>a.lucro-b.lucro || tie(a,b)).slice(0,10);
const pequenas = [
...segmentos.filter(r=>r.total<50 && r.lucro>0).sort((a,b)=>b.lucro-a.lucro || tie(a,b)).slice(0,4),
...segmentos.filter(r=>r.total<50 && r.lucro<0).sort((a,b)=>a.lucro-b.lucro || tie(a,b)).slice(0,4)
];
const section = (title,subtitle,rows) => `<section style="margin-bottom:24px;"><div class="stat-card-title">${title}</div><div class="stat-card-sub">${subtitle}</div>
${rows.length ? `<div style="overflow-x:auto;"><table class="stat-table summary-segments" style="min-width:900px;table-layout:auto;"><thead><tr><th>Esporte / categoria</th><th>Mercado</th><th>Odds</th><th>Apostas</th><th>Volume</th><th>Lucro</th><th>ROI</th><th>Acerto</th></tr></thead><tbody>${rows.map(r=>`<tr><td>${iconeCategoria(r.category)}${escapeHtml(r.category)}</td><td style="white-space:normal;min-width:260px;">${escapeHtml(r.market)}</td><td>${escapeHtml(r.odds)}</td><td>${r.total}</td><td>${fmtEstat(r.volume,false)}</td><td style="color:${color(r.lucro)}">${fmtEstat(r.lucro,true)}</td><td style="color:${color(r.roi)}">${r.roi>0?'+':''}${r.roi.toFixed(2)}%</td><td>${r.taxaAcerto.toFixed(1)}%</td></tr>`).join('')}</tbody></table></div>` : '<p style="color:var(--muted);font-size:.85rem;">Nenhum segmento neste critério para os filtros selecionados.</p>'}</section>`;
return `<div style="padding:4px 0;"><p style="color:var(--muted);font-size:.85rem;margin:0 0 20px;">Esporte/categoria × mercado × faixa de odds. Respeita todos os filtros acima. Usa os mesmos mercados detalhados de “Por Mercado”, sem juntar períodos ou escopos diferentes.</p>
${section('Segmentos com lucro','Até 10 maiores lucros líquidos entre cruzamentos com pelo menos 50 apostas.',lucros)}
${section('Segmentos com perdas','Até 10 maiores prejuízos entre cruzamentos com pelo menos 50 apostas.',perdas)}
${section('Amostras pequenas','Até 4 maiores lucros e 4 maiores prejuízos entre cruzamentos com menos de 50 apostas. Não representam uma tendência confirmada.',pequenas)}
</div>`;
}
function desenharGraficoPatrimonio() {
const wrap = document.getElementById("chartArea");
const valorEl = document.getElementById("chartValorAtual");
const variacaoEl = document.getElementById("chartVariacao");
const labelEl = document.getElementById("chartValorLabel");
const tooltip = document.getElementById("chartTooltip");
if (!wrap || !valorEl || !variacaoEl) return;
if (labelEl) labelEl.textContent = "PnL Acumulado";
variacaoEl.style.display = "none";
const pontos = historicoPnlAcumulado(periodoGraficoAtual);
const atual = pontos.length ? pontos[pontos.length - 1].v : 0;
valorEl.textContent = (atual > 0 ? "+" : "") + fmtDinheiro(atual);
valorEl.className = "chart-valor " + (atual > 0 ? "positive" : atual < 0 ? "negative" : "neutral");
// A série lógica acima continua completa para cálculos. Daqui em diante,
// somente a linha exibida usa um ponto por dia, no fechamento daquele dia.
const pontosDiarios = agruparPnlAcumuladoPorDia(pontos);
if (!pontosDiarios.length) {
wrap.innerHTML = '<div class="chart-empty">Ainda não há posições fechadas nesse período para montar o gráfico.</div>';
variacaoEl.textContent = "";
if (tooltip) tooltip.style.display = "none";
return;
}
// A escala também representa os fechamentos diários efetivamente desenhados.
const vals = pontosDiarios.map(p => p.v);
let min = Infinity, max = -Infinity;
for (const valor of vals) { if (valor < min) min = valor; if (valor > max) max = valor; }
let amplitudeCores = 0;
for (const valor of vals) amplitudeCores = Math.max(amplitudeCores, Math.abs(valor));
if (min === max) { min -= 1; max += 1; }
const folga = (max - min) * 0.12;
min -= folga; max += folga;
const range = max - min;
const w = 700, h = 240;
const padL = 16, padR = 16, padT = 16, padB = 16;
const plotW = w - padL - padR;
const plotH = h - padT - padB;
const plotBottom = padT + plotH;
const stepX = pontosDiarios.length > 1 ? plotW / (pontosDiarios.length - 1) : 0;
const pontosVisuais = pontosVisuaisGraficoAcumulado(pontosDiarios);
const coords = pontosVisuais.map(({ ponto, index }) => {
const x = pontosDiarios.length === 1 ? padL + plotW / 2 : padL + index * stepX;
const v = ponto.v;
const y = padT + (1 - (v - min) / range) * plotH;
return [x, y];
});
const ultimo = vals[vals.length - 1];
const stopsCores = stopsGradientePnl(pontosVisuais, pontosDiarios, amplitudeCores);
const linePath = coords.length === 1
? `M${padL.toFixed(2)},${coords[0][1].toFixed(2)} L${(w - padR).toFixed(2)},${coords[0][1].toFixed(2)}`
: curvaSuaveMonotona(coords);
const areaPath = coords.length === 1
? linePath + ` L${(w - padR).toFixed(2)},${plotBottom} L${padL.toFixed(2)},${plotBottom} Z`
: linePath + ` L${coords[coords.length - 1][0].toFixed(2)},${plotBottom} L${coords[0][0].toFixed(2)},${plotBottom} Z`;
wrap.innerHTML = `
<svg viewBox="0 0 ${w} ${h}" preserveAspectRatio="none" class="chart-svg">
<defs>
<linearGradient id="chartPnlGradient" gradientUnits="userSpaceOnUse" x1="${padL}" y1="0" x2="${w - padR}" y2="0">
${stopsCores}
</linearGradient>
<linearGradient id="chartPnlFillFade" x1="0" y1="0" x2="0" y2="1">
<stop offset="0%" stop-color="#fff" stop-opacity="0.26"/>
<stop offset="100%" stop-color="#fff" stop-opacity="0"/>
</linearGradient>
<mask id="chartPnlFillMask" maskUnits="userSpaceOnUse" x="${padL}" y="${padT}" width="${plotW}" height="${plotH}">
<rect x="${padL}" y="${padT}" width="${plotW}" height="${plotH}" fill="url(#chartPnlFillFade)"/>
</mask>
</defs>
<path d="${areaPath}" fill="url(#chartPnlGradient)" mask="url(#chartPnlFillMask)" stroke="none"/>
<path class="chart-line" d="${linePath}" fill="none" stroke="url(#chartPnlGradient)" stroke-width="2.75" stroke-linejoin="round" stroke-linecap="round"/>
<line class="hover-line" x1="0" y1="${padT}" x2="0" y2="${plotBottom}"/>
<circle class="hover-dot" r="4.5" stroke="${corGradientePnl(ultimo, amplitudeCores)}"/>
<rect class="hit-rect" x="${padL}" y="0" width="${plotW}" height="${h}"/>
</svg>
`;
variacaoEl.textContent = "";
// Animação de "desenhar" a linha
const svgEl = wrap.querySelector(".chart-svg");
const linePathEl = wrap.querySelector(".chart-line");
if (linePathEl && linePathEl.getTotalLength) {
const len = linePathEl.getTotalLength();
linePathEl.style.strokeDasharray = len;
linePathEl.style.strokeDashoffset = len;
requestAnimationFrame(() => {
linePathEl.style.transition = "stroke-dashoffset 900ms ease";
linePathEl.style.strokeDashoffset = "0";
});
}
// Hover: a linha desenhada pode ser projetada, mas tooltip e valor sempre
// apontam para o ponto exato da série completa.
const hitRect = wrap.querySelector(".hit-rect");
const hoverLine = wrap.querySelector(".hover-line");
const hoverDot = wrap.querySelector(".hover-dot");
if (hitRect && hoverLine && hoverDot && svgEl) {
hitRect.addEventListener("mousemove", (e) => {
const rect = svgEl.getBoundingClientRect();
const mx = ((e.clientX - rect.left) / rect.width) * w;
const relativo = stepX > 0 ? (mx - padL) / stepX : 0;
const idx = Math.max(0, Math.min(pontosDiarios.length - 1, Math.ceil(relativo - 0.5)));
const ponto = pontosDiarios[idx];
const x = pontosDiarios.length === 1 ? padL + plotW / 2 : padL + idx * stepX;
const c = [x, padT + (1 - (ponto.v - min) / range) * plotH];
hoverLine.setAttribute("x1", c[0]); hoverLine.setAttribute("x2", c[0]);
hoverLine.style.display = "block";
hoverDot.setAttribute("cx", c[0]); hoverDot.setAttribute("cy", c[1]);
hoverDot.setAttribute("stroke", corGradientePnl(ponto.v, amplitudeCores));
hoverDot.style.display = "block";
if (tooltip) {
const p = ponto;
desenharGraficoPatrimonio.formatoTooltip ||= new Intl.DateTimeFormat("pt-BR", { day: "2-digit", month: "2-digit", year: "2-digit", timeZone: "America/Sao_Paulo" });
const dt = desenharGraficoPatrimonio.formatoTooltip.format(new Date(p.t));
const pct = Math.min(94, Math.max(6, (c[0] / w) * 100));
tooltip.style.display = "block";
tooltip.style.left = pct + "%";
tooltip.textContent = dt + " • " + fmtDinheiro(p.v);
}
});
hitRect.addEventListener("mouseleave", () => {
hoverLine.style.display = "none";
hoverDot.style.display = "none";
if (tooltip) tooltip.style.display = "none";
});
}
}
// ===== PnL por Dia (gráfico de barras) =====
function bucketsPnlPorDia(periodoDias) {
const cfgDb = chavePnlHistoricoDb();
if (indiceDisponivel() && cfgDb.mode === "day" && pnlHistoricoDb?.key === cfgDb.key) return pnlHistoricoDb.serie;
const agora = new Date();
const quantidadeDias = Number.isInteger(periodoDias) && periodoDias > 0 ? periodoDias : 30;
// Monta exatamente os últimos N dias civis, incluindo hoje, no horário de
// Brasília. Para 30, o primeiro bucket é hoje menos 29 dias.
const dias = [];
const fimYmd = ymdBrasil(agora);
const [fy, fm, fd] = fimYmd.split("-").map(Number);
const cur = new Date(fy, fm - 1, fd);
cur.setDate(cur.getDate() - (quantidadeDias - 1));
for (let i = 0; i < quantidadeDias; i++) {
const y = cur.getFullYear();
const m = String(cur.getMonth() + 1).padStart(2, "0");
const day = String(cur.getDate()).padStart(2, "0");
dias.push(`${y}-${m}-${day}`);
cur.setDate(cur.getDate() + 1);
}
const somaPorDia = {};
dias.forEach(d => somaPorDia[d] = 0);
fechadosDoGrafico().forEach(d => {
// A data do PnL vem da resolução/liquidação ou venda antecipada.
const ts = getTsResultadoPnL(d);
if (!ts) return;
const ymd = ymdBrasil(new Date(ts));
if (somaPorDia[ymd] != null) somaPorDia[ymd] += (d.realizedPnl || 0);
});
return dias.map(d => ({ d, v: somaPorDia[d] }));
}
// ===== Gráficos de barras (PnL por Dia / por Mês) =====
// O SVG usa o tamanho que ocupa na tela (viewBox = pixels reais). Com um viewBox fixo
// e preserveAspectRatio="none", números, linhas e o ponto do hover esticavam junto.
// Por isso o gráfico é redesenhado quando a área muda de tamanho (observarTamanhoGrafico).

// Escala "redonda" para o eixo Y: passo 1/2/2,5/5 × 10^n, sempre passando pelo zero.
function escalaEixoPnl(lo, hi) {
if (lo === hi) { lo -= 1; hi += 1; }
const bruto = (hi - lo) / 4;
const mag = 10 ** Math.floor(Math.log10(bruto));
const passo = [1, 2, 2.5, 5, 10].map(m => m * mag).find(p => p >= bruto * 0.999);
let min = Math.floor(lo / passo) * passo, max = Math.ceil(hi / passo) * passo;
// Folga para a barra mais alta não encostar no limite do gráfico.
if (hi > 0 && max - hi < passo * 0.15) max += passo;
if (lo < 0 && lo - min < passo * 0.15) min -= passo;
const marcas = [];
for (let k = Math.round(min / passo); k <= Math.round(max / passo); k++) marcas.push(k * passo);
return { min, max, passo, marcas };
}
function fmtEixoPnl(v, passo) {
if (Math.abs(v) < passo / 1000) return "$0";
const casas = Number.isInteger(passo) ? 0 : 2;
const abs = Math.abs(v).toLocaleString("en-US", { minimumFractionDigits: casas, maximumFractionDigits: casas });
return (v < 0 ? "-$" : "$") + abs;
}
function desenharBarrasPnl(wrap, itens, { rotulo, fracBarra, barraMin }) {
const tooltip = document.getElementById("chartTooltip");
wrap.innerHTML = '<svg class="chart-svg" aria-hidden="true"></svg><div class="chart-labels"></div>';
const svgEl = wrap.querySelector(".chart-svg");
const caixa = svgEl.getBoundingClientRect();
const w = Math.max(240, Math.round(caixa.width)), h = Math.max(140, Math.round(caixa.height));
svgEl.setAttribute("viewBox", `0 0 ${w} ${h}`);
const valores = itens.map(d => d.v);
const { min, max, passo, marcas } = escalaEixoPnl(Math.min(0, ...valores), Math.max(0, ...valores));
// Coluna dos valores do eixo: largura do maior rótulo, medida com a fonte real.
svgEl.innerHTML = marcas.map(v => `<text class="grid-label">${fmtEixoPnl(v, passo)}</text>`).join("");
const larguraRotulos = Math.max(...[...svgEl.querySelectorAll(".grid-label")].map(t => t.getComputedTextLength()));
const padL = Math.ceil(larguraRotulos) + 14, padR = 14, padT = 12, padB = 10;
const plotW = w - padL - padR, plotH = h - padT - padB, plotBottom = padT + plotH;
const yDe = v => padT + (1 - (v - min) / (max - min)) * plotH;
const linha = y => Math.round(y) + 0.5; // linha de 1px nítida
const zeroY = Math.round(yDe(0));
const slotW = plotW / itens.length;
const barW = Math.max(barraMin, Math.round(slotW * fracBarra));
const centros = itens.map((d, i) => padL + (i + 0.5) * slotW);
const corDe = v => v > 0 ? "#3ddc84" : v < 0 ? "#ff6369" : "#3a4150";
const grade = marcas.map(v => {
const y = yDe(v);
return (v === 0 ? "" : `<line class="grid-line" x1="${padL}" y1="${linha(y)}" x2="${w - padR}" y2="${linha(y)}"/>`)
+ `<text class="grid-label" x="${padL - 10}" y="${Math.round(y)}" text-anchor="end" dominant-baseline="central">${fmtEixoPnl(v, passo)}</text>`;
}).join("");
const barras = itens.map((d, i) => {
const topo = Math.round(Math.min(yDe(d.v), zeroY)), base = Math.round(Math.max(yDe(d.v), zeroY));
const alt = Math.max(1, base - topo);
const x = Math.round(centros[i] - barW / 2);
return `<rect class="pnl-bar" x="${x}" width="${barW}" y="${zeroY}" height="0" data-y="${topo}" data-h="${alt}" rx="2.5" fill="${corDe(d.v)}"/>`;
}).join("");
svgEl.innerHTML = `${grade}${barras}
<line class="zero-line" x1="${padL}" y1="${zeroY + 0.5}" x2="${w - padR}" y2="${zeroY + 0.5}"/>
<line class="hover-line" x1="0" y1="${padT}" x2="0" y2="${plotBottom}"/>
<circle class="hover-dot" r="4.5"/>
<rect class="hit-rect" x="${padL}" y="0" width="${plotW}" height="${h}"/>`;
const labelsEl = wrap.querySelector(".chart-labels");
const labelsIdx = [...new Set([0, Math.floor((itens.length - 1) / 2), itens.length - 1])];
labelsEl.innerHTML = labelsIdx.map(i => `<span>${rotulo(itens[i].d)}</span>`).join("");
labelsEl.style.setProperty("padding-left", padL + "px", "important");
labelsEl.style.setProperty("padding-right", padR + "px", "important");
requestAnimationFrame(() => {
svgEl.querySelectorAll(".pnl-bar").forEach(el => {
el.setAttribute("y", el.dataset.y);
el.setAttribute("height", el.dataset.h);
});
});
const hitRect = svgEl.querySelector(".hit-rect");
const hoverLine = svgEl.querySelector(".hover-line");
const hoverDot = svgEl.querySelector(".hover-dot");
hitRect.addEventListener("mousemove", (e) => {
const mx = e.clientX - svgEl.getBoundingClientRect().left;
const idx = Math.max(0, Math.min(itens.length - 1, Math.floor((mx - padL) / slotW)));
const cx = Math.round(centros[idx]) + 0.5;
const v = itens[idx].v;
hoverLine.setAttribute("x1", cx); hoverLine.setAttribute("x2", cx);
hoverLine.style.display = "block";
hoverDot.setAttribute("cx", cx); hoverDot.setAttribute("cy", yDe(v));
hoverDot.setAttribute("stroke", corDe(v));
hoverDot.style.display = "block";
if (tooltip) {
tooltip.style.display = "block";
tooltip.style.left = Math.min(94, Math.max(6, (cx / w) * 100)) + "%";
tooltip.textContent = rotulo(itens[idx].d) + " • " + (v >= 0 ? "+" : "") + fmtDinheiro(v);
}
});
hitRect.addEventListener("mouseleave", () => {
hoverLine.style.display = "none";
hoverDot.style.display = "none";
if (tooltip) tooltip.style.display = "none";
});
}
// Redesenha as barras quando a área do gráfico muda de tamanho (janela, sidebar).
// A curva do PnL Acumulado não tem texto dentro do SVG e continua esticando sozinha.
function observarTamanhoGrafico() {
const area = document.getElementById("chartArea");
if (!area || typeof ResizeObserver !== "function") return;
let largura = 0, altura = 0, agendado = false;
new ResizeObserver(([entrada]) => {
const { width, height } = entrada.contentRect;
if (Math.round(width) === largura && Math.round(height) === altura) return;
const primeira = largura === 0;
largura = Math.round(width); altura = Math.round(height);
if (primeira || agendado || (tipoGraficoAtual !== "pnldia" && tipoGraficoAtual !== "pnlmes")) return;
agendado = true;
requestAnimationFrame(() => { agendado = false; if (dashboardEstaVisivel()) renderizarGraficoComSerie(); });
}).observe(area);
}
observarTamanhoGrafico();
// ===== PnL por Dia (barras) =====
function desenharGraficoPnlDia() {
const wrap = document.getElementById("chartArea");
const valorEl = document.getElementById("chartValorAtual");
const variacaoEl = document.getElementById("chartVariacao");
const labelEl = document.getElementById("chartValorLabel");
const tooltip = document.getElementById("chartTooltip");
if (!wrap || !valorEl || !variacaoEl) return;
variacaoEl.style.display = "";
if (labelEl) labelEl.textContent = "PnL no Período";
const dias = bucketsPnlPorDia(30);
const totalPeriodo = dias.reduce((s, d) => s + d.v, 0);
valorEl.textContent = (totalPeriodo > 0 ? "+" : "") + fmtDinheiro(totalPeriodo);
valorEl.className = "chart-valor " + (totalPeriodo > 0 ? "positive" : totalPeriodo < 0 ? "negative" : "neutral");
variacaoEl.className = "chart-variacao " + (totalPeriodo >= 0 ? "positive" : "negative");
const comMovimento = dias.filter(d => d.v !== 0);
variacaoEl.textContent = comMovimento.length
? `${comMovimento.filter(d => d.v > 0).length}/${comMovimento.length} dias no lucro`
: "";
if (!dias.length) {
wrap.innerHTML = '<div class="chart-empty">Sem dados nesse período.</div>';
if (tooltip) tooltip.style.display = "none";
return;
}
const fmtLabel = (ymd) => { const [, mm, dd] = ymd.split("-"); return `${dd}/${mm}`; };
desenharBarrasPnl(wrap, dias, { rotulo: fmtLabel, fracBarra: dias.length > 40 ? 0.7 : 0.55, barraMin: 2 });
}
// ===== PnL por Mês (barras) =====
function ymBrasil(dt) {
if (!dt) return null;
ymBrasil.formato ||= new Intl.DateTimeFormat("en-CA", {
timeZone: "America/Sao_Paulo",
year: "numeric",
month: "2-digit",
});
return ymBrasil.formato.format(dt); // YYYY-MM
}
function bucketsPnlPorMes() {
const cfgDb = chavePnlHistoricoDb();
if (indiceDisponivel() && cfgDb.mode === "month" && pnlHistoricoDb?.key === cfgDb.key) return pnlHistoricoDb.serie;
const soma = {};
fechadosDoGrafico().forEach(d => {
const ts = getTsResultadoPnL(d);
if (!ts) return;
const ym = ymBrasil(new Date(ts));
if (!ym) return;
soma[ym] = (soma[ym] || 0) + (d.realizedPnl || 0);
});
const keys = Object.keys(soma).sort();
if (!keys.length) {
const agora = ymBrasil(new Date());
return agora ? [{ d: agora, v: 0 }] : [];
}
// Preenche meses vazios entre o primeiro e o último
const out = [];
const [y0, m0] = keys[0].split("-").map(Number);
const [y1, m1] = keys[keys.length - 1].split("-").map(Number);
let y = y0, m = m0;
while (y < y1 || (y === y1 && m <= m1)) {
const k = y + "-" + String(m).padStart(2, "0");
out.push({ d: k, v: soma[k] || 0 });
m += 1;
if (m > 12) { m = 1; y += 1; }
}
return out;
}
function desenharGraficoPnlMes() {
const wrap = document.getElementById("chartArea");
const valorEl = document.getElementById("chartValorAtual");
const variacaoEl = document.getElementById("chartVariacao");
const labelEl = document.getElementById("chartValorLabel");
const tooltip = document.getElementById("chartTooltip");
if (!wrap || !valorEl || !variacaoEl) return;
variacaoEl.style.display = "";
if (labelEl) labelEl.textContent = "PnL por Mês";
const meses = bucketsPnlPorMes();
const total = meses.reduce((s, d) => s + d.v, 0);
valorEl.textContent = (total > 0 ? "+" : "") + fmtDinheiro(total);
valorEl.className = "chart-valor " + (total > 0 ? "positive" : total < 0 ? "negative" : "neutral");
variacaoEl.className = "chart-variacao " + (total >= 0 ? "positive" : "negative");
const comMov = meses.filter(d => d.v !== 0);
variacaoEl.textContent = comMov.length
? (comMov.filter(d => d.v > 0).length + "/" + comMov.length + " meses no lucro")
: "";
if (!meses.length) {
wrap.innerHTML = '<div class="chart-empty">Sem dados mensais.</div>';
if (tooltip) tooltip.style.display = "none";
return;
}
const fmtMes = (ym) => { const [yy, mm] = ym.split("-"); return mm + "/" + yy.slice(2); };
desenharBarrasPnl(wrap, meses, { rotulo: fmtMes, fracBarra: 0.55, barraMin: 6 });
}
// ===== Status "Online" + "Atualizado há Xs" =====
// Enquanto true, o texto abaixo de "Online" mostra "Atualizando..." e o
// tick de 1s (tickAtualizado) não sobrescreve essa mensagem.
let mostrandoAtualizando = false;
function mensagemErroSync(error) {
if (!error) return "Falha ao consultar a casa.";
const t = String(error.message || error);
if (/HTTP 401|Credenciais/i.test(t)) return "Credenciais recusadas.";
if (/HTTP 409|Conecte/i.test(t)) return "Conta não conectada.";
if (/HTTP 429/i.test(t)) return "Limite de requisições. Tente de novo em instantes.";
if (/Failed to fetch|NetworkError|timeout|Abort/i.test(t)) return "Falha de rede ou tempo esgotado.";
return t.slice(0, 140);
}
function registrarSyncCasa(casa, patch) {
casaSync[casa] = { ...casaSyncPadrao(casa), ...casaSync[casa], ...patch, casa };
salvarSyncPersistido();
}
function casasConectadas() {
return CASAS_STAKE.filter(casa => BETTY_CONNECTIONS?.[casa]);
}
function syncEstaStale() {
const casas = casasConectadas();
if (!casas.length) return false;
return casas.some(c => casaSync[c]?.state === "error" || casaSync[c]?.state === "stale");
}
function fmtHoraSync(ts) {
if (!ts) return "nunca";
const d = new Date(ts);
if (Number.isNaN(d.getTime())) return "nunca";
return d.toLocaleString("pt-BR", { day: "2-digit", month: "2-digit", hour: "2-digit", minute: "2-digit", timeZone: "America/Sao_Paulo" });
}
function rotuloEstadoSync(state) {
if (state === "ok") return "Confirmado";
if (state === "syncing") return "Atualizando";
if (state === "error") return "Erro";
if (state === "stale") return "Desatualizado";
if (state === "partial") return "Histórico parcial";
if (state === "disconnected") return "Desconectada";
return "Aguardando";
}
function renderSyncPanel() {
const el = document.getElementById("syncPanel");
if (!el) return;
el.innerHTML = Object.entries(NOMES_CASAS).map(([id, nome]) => {
const s = casaSync[id] || casaSyncPadrao(id);
const connected = Boolean(BETTY_CONNECTIONS?.[id]);
const state = connected ? (s.state || "idle") : "disconnected";
const counts = connected ? `${s.openCount || 0} abertas` + (s.closedCount ? ` · ${s.closedCount} fechadas recebidas` : "") : "Não conectada";
return `<div class="sync-house">
<div class="name"><span class="status-dot ${state === "ok" ? "" : state === "error" ? "err" : ["stale", "syncing", "partial"].includes(state) ? "warn" : "off"}"></span>${nome} · ${rotuloEstadoSync(state)}</div>
<div class="meta">Último sync bem-sucedido: ${fmtHoraSync(s.lastOk)}<br>${counts}${s.state === "stale" ? "<br>Exibindo dados anteriores." : ""}</div>
${s.lastError ? `<div class="err">${escapeHtml(s.lastError)}</div>` : ""}
</div>`;
}).join("");
}
function toggleSyncPanel(ev) {
if (ev) ev.stopPropagation();
syncPanelAberto = !syncPanelAberto;
document.getElementById('statusBox')?.setAttribute('aria-expanded',String(syncPanelAberto));
const panel = document.getElementById("syncPanel");
if (!panel) return;
if (syncPanelAberto) {
renderSyncPanel();
panel.classList.add("open");
setTimeout(() => document.addEventListener("click", fecharSyncPanelFora), 0);
} else {
panel.classList.remove("open");
document.removeEventListener("click", fecharSyncPanelFora);
}
}
function fecharSyncPanelFora(ev) {
const wrap = document.querySelector(".online-wrap");
if (wrap && !wrap.contains(ev.target)) {
syncPanelAberto = false;
document.getElementById('statusBox')?.setAttribute('aria-expanded','false');
document.getElementById("syncPanel")?.classList.remove("open");
document.removeEventListener("click", fecharSyncPanelFora);
}
}
function atualizarBannerStale() {
const banner = document.getElementById("staleBanner");
const text = document.getElementById("staleBannerText");
if (!banner || !text) return;
const avisos = casasConectadas().filter(casa => casaSync[casa]?.state === "partial")
.map(casa => casaSync[casa].lastError).filter(Boolean);
if (!syncEstaStale() && !avisos.length) {
banner.classList.remove("show");
return;
}
const falhas = casasConectadas().filter(c => casaSync[c]?.state === "error" || casaSync[c]?.state === "stale");
const nomes = falhas.map(c => NOMES_CASAS[c]).join(", ");
const ts = Math.max(...casasConectadas().map(c => Number(casaSync[c]?.lastOk) || 0));
const anteriores = falhas.length ? `<strong>Dados desatualizados.</strong> A última atualização de ${escapeHtml(nomes)} falhou. ${ts ? "Última confirmação em " + fmtHoraSync(ts) + "." : ""}` : "";
text.innerHTML = [anteriores, ...avisos.map(escapeHtml)].filter(Boolean).join(" ");
banner.classList.add("show");
}
function atualizarStatusOnline(ok) {
const dot = document.getElementById("statusDot");
const label = document.getElementById("statusOnline");
const casas = casasConectadas();
const stale = syncEstaStale();
const syncing = casas.some(c => casaSync[c]?.state === "syncing") || mostrandoAtualizando;
let texto = "Sem contas";
let cls = "off";
if (!casas.length) {
texto = "Sem contas";
} else if (syncing) {
texto = "Atualizando";
cls = "warn";
} else if (stale && casas.every(c => casaSync[c]?.state === "error")) {
texto = "Falha no sync";
cls = "err";
} else if (stale) {
texto = "Dados desatualizados";
cls = "warn";
} else if (casas.some(casa => casaSync[casa]?.state === "partial")) {
texto = "Histórico parcial";
cls = "warn";
} else if (ok || casas.every(c => casaSync[c]?.state === "ok")) {
texto = "Dados confirmados";
cls = "";
} else {
texto = "Aguardando sync";
cls = "off";
}
if (dot) {
dot.classList.remove("off", "warn", "err");
if (cls) dot.classList.add(cls);
}
if (label) label.textContent = texto;
if (ok && !stale) ultimoFetchTs = Date.now();
mostrandoAtualizando = false;
tickAtualizado();
atualizarBannerStale();
if (syncPanelAberto) renderSyncPanel();
}
function tickAtualizado() {
renderSyncCasas();
const el = document.getElementById("statusAtualizado");
if (!el) return;
if (mostrandoAtualizando) return;
const ts = ultimoFetchTs || Math.max(0, ...Object.values(casaSync).map(s => Number(s.lastOk) || 0));
if (!ts) { el.textContent = "—"; return; }
const stale = syncEstaStale();
const diffS = Math.max(0, Math.round((Date.now() - ts) / 1000));
const base = diffS < 60 ? `há ${diffS}s` : `há ${Math.floor(diffS / 60)}min`;
const completo=stale ? `Snapshot ${base}` : `Confirmado ${base}`;
el.textContent = el.closest?.('#sidebarSyncStatusSlot') ? (diffS<60?`${diffS}s`:`${Math.floor(diffS/60)}min`) : completo;
el.setAttribute?.('aria-label',completo);
}
// ===== Auto-refresh =====
function iniciarAutoRefresh() {
// Nunca mantenha um timer ativo enquanto a função estiver desligada.
if (!autoRefreshAtivo) {
if (autoRefreshTimer) {
clearInterval(autoRefreshTimer);
autoRefreshTimer = null;
}
return;
}
if (autoRefreshTimer) clearInterval(autoRefreshTimer);
autoRefreshTimer = setInterval(() => {
// Dupla proteção: mesmo que um callback já esteja enfileirado, ele não
// pode atualizar nada depois que o usuário pausar a função.
if (!autoRefreshAtivo || carregandoAba) return;
const abaDados = abaAtual === "ao-vivo" ? "aberto" : abaAtual;
carregarAba(abaDados);
}, 60000);
}
function atualizarIndicadorAutoRefresh() {
const label = document.getElementById("autoRefreshLabel");
const dot = document.getElementById("autoRefreshDot");
if (label) label.textContent = autoRefreshAtivo ? "Ativado (60s)" : "Pausado";
if (dot) dot.classList.toggle("off", !autoRefreshAtivo);
document.getElementById('autoRefreshToggle')?.setAttribute('aria-pressed',String(autoRefreshAtivo));
}
function alternarAutoRefresh() {
autoRefreshAtivo = !autoRefreshAtivo;
try { localStorage.setItem("betty:autoRefresh", autoRefreshAtivo ? "1" : "0"); } catch (_) {}
if (autoRefreshAtivo) iniciarAutoRefresh();
else if (autoRefreshTimer) {
clearInterval(autoRefreshTimer);
autoRefreshTimer = null;
}
atualizarIndicadorAutoRefresh();
}
function atualizarAgora() {
// Atualização incremental: nunca recarrega a página nem limpa a tabela.
// Os dados já exibidos permanecem visíveis enquanto a API é consultada;
// quando o novo snapshot é confirmado, ele é aplicado e o histórico
// persistente continua sendo mesclado com as novas fechadas.
if (carregandoAba) return;
const abaDados = (abaAtual === "ao-vivo") ? "aberto" : abaAtual;
carregarAba(abaDados, { forcar: true });
}
function posAtualizarDados() {
const positionBadge=document.getElementById('sidebarPositionCount');
// Nos dados salvos, lista vazia pode ser só falta de cache: o zero espera a confirmação.
if (positionBadge) { positionBadge.hidden=!(snapshotAoVivoConfirmado || (exibindoDadosSalvos && abertos.length)); positionBadge.textContent=String(abertos.length); }
if (viewAtual === "calendario") renderProfitCalendar();
atualizarFiltroEsportePosicoes();
// Ao vivo é apenas uma visualização derivada das posições.
// Após qualquer refresh, nunca renderizar a lista normal/fechada por cima dela.
if (viewAtual === "posicoes" && abaAtual === "ao-vivo") {
renderAoVivo({ manterPagina: true });
} else if (viewAtual === "posicoes") {
aplicarFiltros({ manterPagina: true });
} else {
// Prepara os resumos sem filtrar, ordenar ou redesenhar a tabela oculta.
renderizarStats();
}
desenharGrafico();
if (viewAtual === "estatisticas") renderEstatisticas();
}

// Ações com lógica própria, chamadas pelos atributos data-on-* (o CSP não
// permite script embutido no HTML; ver acoes.js).
function aplicarFiltrosSemOrdem() { sortCol = null; aplicarFiltros(); }
function recarregarAbaAtual() { carregarAba(abaAtual, { forcar: true }); }
function teclaTrocarView(event, view) {
  if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); trocarView(view); }
}
function teclaToggleSyncPanel(event) {
  if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); toggleSyncPanel(event); }
}

// ===== Navegação por seção (layout v2) =====
// As seções do menu se apoiam nas views existentes: Histórico usa a tabela de
// Posições na aba "fechado" (Lista) ou o calendário de lucro (Calendário), e
// Conexões e Configurações dividem a view de configuração.
const TITULOS_SECAO = {
  visao: ["Visão geral", "Resultados no horário de São Paulo"],
  posicoes: ["Posições", "Apostas em aberto e ao vivo"],
  historico: ["Histórico", "Apostas fechadas, em lista ou no calendário"],
  estatisticas: ["Estatísticas", "Desempenho das posições fechadas"],
  conexoes: ["Conexões", "Contas conectadas, sempre só para leitura"],
  config: ["Configurações", "Stake e período do histórico por casa"],
};
let secaoAtual = "visao";
let secaoPedida = null;
let modoHistorico = "lista";
function irPara(secao) {
  if (!TITULOS_SECAO[secao]) return;
  secaoPedida = secao;
  try {
    if (secao === "visao") trocarView("dashboard");
    else if (secao === "posicoes") {
      trocarView("posicoes");
      if (viewAtual === "posicoes" && abaAtual === "fechado") trocarAba("aberto");
    } else if (secao === "historico") {
      if (modoHistorico === "calendario") trocarView("calendario");
      else {
        trocarView("posicoes");
        if (viewAtual === "posicoes") trocarAba("fechado");
      }
    } else if (secao === "estatisticas") trocarView("estatisticas");
    else trocarView("config");
  } finally {
    secaoPedida = null;
  }
}
function teclaIrPara(event, secao) {
  if (event.key === "Enter" || event.key === " ") { event.preventDefault(); irPara(secao); }
}
function trocarModoHistorico(modo) {
  if (modo !== "lista" && modo !== "calendario") return;
  modoHistorico = modo;
  irPara("historico");
}
// Chamada por trocarView depois de trocar a view: decide a seção, o item ativo
// do menu e o título da tela.
function atualizarSecaoNavegacao(view) {
  const derivada = {
    dashboard: "visao",
    posicoes: abaAtual === "fechado" ? "historico" : "posicoes",
    calendario: "historico",
    estatisticas: "estatisticas",
    config: secaoAtual === "conexoes" ? "conexoes" : "config",
  }[view];
  const secao = secaoPedida || derivada || "visao";
  secaoAtual = secao;
  document.body.dataset.secao = secao;
  document.querySelectorAll(".nav-item[data-secao]").forEach(el => {
    const ativo = el.dataset.secao === secao;
    el.classList.toggle("active", ativo);
    if (ativo) el.setAttribute("aria-current", "page");
    else el.removeAttribute("aria-current");
  });
  const [titulo, sub] = TITULOS_SECAO[secao];
  const h1 = document.getElementById("topbarTitle");
  const subEl = document.getElementById("topbarSub");
  if (h1) h1.textContent = titulo;
  if (subEl) subEl.textContent = sub;
  document.querySelectorAll("[data-modo-historico]").forEach(b => {
    const ativo = b.dataset.modoHistorico === (view === "calendario" ? "calendario" : "lista");
    b.classList.toggle("on", ativo);
    b.setAttribute("aria-pressed", String(ativo));
  });
}
// Rodapé do menu: uma linha por casa com logo, estado e tempo desde o último sync.
function tempoDesde(ts) {
  if (!ts) return "nunca";
  const s = Math.max(0, Math.round((Date.now() - ts) / 1000));
  if (s < 60) return "agora";
  if (s < 3600) return `há ${Math.floor(s / 60)} min`;
  if (s < 86400) return `há ${Math.floor(s / 3600)} h`;
  return `há ${Math.floor(s / 86400)} d`;
}
function renderSyncCasas() {
  const el = document.getElementById("syncCasas");
  if (!el) return;
  const html = Object.entries(NOMES_CASAS).map(([id, nome]) => {
    const s = casaSync[id] || casaSyncPadrao(id);
    const conectada = Boolean(BETTY_CONNECTIONS?.[id]);
    const estado = conectada ? (s.state || "idle") : "disconnected";
    const ponto = { ok: "ok", syncing: "run", error: "err", stale: "warn", partial: "warn" }[estado] || "";
    const texto = !conectada ? "não conectada"
      : estado === "ok" ? tempoDesde(s.lastOk)
      : estado === "syncing" ? "atualizando"
      : estado === "error" ? "falhou"
      : estado === "stale" ? "desatualizada"
      : estado === "partial" ? "parcial"
      : "aguardando";
    return `<button type="button" class="sync-casa${conectada ? "" : " off"}" data-on-click="irPara('conexoes')" title="${escapeHtml(nome)}: ${escapeHtml(rotuloEstadoSync(estado))}"><img src="/assets/platforms/${id}.png" alt="">${escapeHtml(nome)}<span class="st"><span class="dot ${ponto}"></span>${texto}</span></button>`;
  }).join("");
  if (el.__html !== html) { el.innerHTML = html; el.__html = html; }
}

// ===== Visão geral (layout v2) =====
// Usa os mesmos números de renderizarStats; aqui só a apresentação.
function fmtDolar(v) {
  const n = Number(v) || 0;
  return (n < 0 ? "-$" : "$") + Math.abs(n).toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
}
function fmtDolarSinal(v) {
  const n = Number(v) || 0;
  return (n >= 0 ? "+" : "") + fmtDolar(n);
}
function classeSinal(v) { return (Number(v) || 0) >= 0 ? "pos" : "neg"; }
function kpisVazios(texto) {
  return ["PnL realizado", "PnL não realizado", "Exposição", "ROI"].map((k, i) =>
    `<div class="fx kpi-vazio"><div class="k">${k}</div><div class="v2">—</div>${i ? "" : `<div class="sm mu">${escapeHtml(texto)}</div>`}</div>`).join("");
}
function saldoPorCasa() {
  return { polymarket: POLY_API_BALANCE, kalshi: KALSHI_API_BALANCE, predict: PREDICT_API_BALANCE, sxbet: SXBET_API_BALANCE };
}
function pintarVisaoGeral(d) {
  const rotulo = document.getElementById("heroPatrimonioRotulo");
  const valor = document.getElementById("heroPatrimonio");
  const chips = document.getElementById("heroChips");
  if (rotulo) rotulo.textContent = d.patrimonioParcial ? "Patrimônio acompanhado" : "Patrimônio";
  if (valor) valor.textContent = d.patrimonioParcial && !d.casasPatrimonio.length ? "—" : fmtDolar(d.patrimonio);
  if (chips) {
    const partes = [
      `<span class="chip ${classeSinal(d.pnlHoje)}">${d.pnlHoje >= 0 ? "▲" : "▼"} ${fmtDolarSinal(d.pnlHoje)} hoje</span>`,
      `<span class="chip ${classeSinal(d.pnlPeriodo)}">${fmtDolarSinal(d.pnlPeriodo)} em 7 dias${d.pnlAnterior == null ? "" : ` · antes ${fmtDolarSinal(d.pnlAnterior)}`}</span>`,
      d.patrimonioParcial
        ? `<span class="sm mu">Inclui ${escapeHtml(d.casasPatrimonio.map(c => NOMES_CASAS[c]).join(", ") || "nenhuma casa")}</span>`
        : `<span class="sm mu">Saldo <b>${fmtDolar(d.saldoPatrimonio)}</b> · em aberto <b>${fmtDolar(d.valorAberto)}</b></span>`,
    ];
    chips.innerHTML = partes.join("");
  }
  pintarCasasVisao(d);
  pintarKpisVisao(d);
  pintarDestaquesVisao(d);
  carregarSeriesVisao().then(ok => { if (ok) pintarMiniGraficosVisao(); });
}
function pintarCasasVisao(d) {
  const el = document.getElementById("visaoCasas");
  if (!el) return;
  const saldos = saldoPorCasa();
  const pnl7 = new Map((d.periodoDb?.porSite || []).map(r => [r.chave, Number(r.lucroD) || 0]));
  const casas = d.casasAtivas;
  if (!casas.length) {
    el.innerHTML = `<div class="card visao-casa-vazia"><b>Nenhuma casa conectada</b><span class="sm mu">Conecte uma conta para acompanhar saldo e posições.</span><button type="button" class="btn" data-on-click="irPara('conexoes')">Abrir Conexões</button></div>`;
    return;
  }
  el.innerHTML = casas.map(casa => {
    const nome = NOMES_CASAS[casa] || casa;
    const grupo = d.casasAbertas[casa] || { count: 0, stake: 0 };
    const saldo = saldos[casa];
    const v7 = pnl7.get(casa) || 0;
    const estado = casaSync[casa]?.state;
    const aviso = estado === "partial" ? `<span class="chip wn">parcial</span>`
      : estado === "error" ? `<span class="chip neg">falhou</span>`
      : estado === "stale" ? `<span class="chip wn">desatualizada</span>` : "";
    return `<button type="button" class="card visao-casa" data-on-click="abrirCasaNaVisao('${casa}')" aria-label="Ver posições da ${escapeHtml(nome)}">
<div class="visao-casa-topo"><img class="lg l" src="/assets/platforms/${casa}.png" alt="">
<div class="grow"><div class="b row-gap">${escapeHtml(nome)}${aviso}</div><div class="sm mu">${grupo.count} ${grupo.count === 1 ? "aberta" : "abertas"} · ${fmtDolar(grupo.stake)} em risco</div></div>
<div class="visao-casa-valor"><div class="v2">${Number.isFinite(saldo) ? fmtDolar(saldo) : "—"}</div><div class="sm ${classeSinal(v7)}">${fmtDolarSinal(v7)} · 7d</div></div></div>
<svg class="visao-spark" data-spark="${casa}" viewBox="0 0 120 26" preserveAspectRatio="none" aria-hidden="true"></svg><div class="visao-spark-legenda">PnL acumulado · últimos 30 dias</div>
</button>`;
  }).join("");
}
function abrirCasaNaVisao(casa) {
  irPara("posicoes");
  if (viewAtual === "posicoes" && typeof selecionarCasa === "function") selecionarCasa(casa);
}
function pintarKpisVisao(d) {
  const el = document.getElementById("stats");
  if (!el) return;
  const soSxBet = d.abertosF.length && !d.abertosF.some(a => a._platform !== "sxbet");
  const comCotacao = d.abertosF.filter(a => a._platform !== "sxbet").length;
  // Faixa: os quatro números calculados aqui; taxa de acerto e odd média
  // são células fixas do HTML, preenchidas por renderizarStats.
  el.innerHTML = `
<div class="fx"><div class="k">PnL realizado</div><div class="v2 ${classeSinal(d.pnlRealizado)}">${fmtDolarSinal(d.pnlRealizado)}</div></div>
<div class="fx"><div class="k">PnL não realizado</div><div class="v2 ${soSxBet ? "" : classeSinal(d.pnlNaoRealizado)}">${soSxBet ? "—" : fmtDolarSinal(d.pnlNaoRealizado)}</div></div>
<div class="fx"><div class="k">Exposição</div><div class="v2">${fmtDolar(d.stakeAbertas)}</div></div>
<div class="fx"><div class="k">ROI</div><div class="v2 ${classeSinal(d.roiGeral)}">${(d.roiGeral >= 0 ? "+" : "") + d.roiGeral.toFixed(2)}%</div></div>`;
  const texto = (id, valor) => { const alvo = document.getElementById(id); if (alvo) alvo.textContent = valor; };
  texto("visaoFechadas", `${d.nFechadas.toLocaleString("en-US")} apostas fechadas no total`);
  texto("visaoCotacao", d.abertosF.length ? `${comCotacao} de ${d.abertosF.length}` : "—");
  texto("visaoCotacaoNota", BETTY_CONNECTIONS?.sxbet ? "A SX Bet informa só a exposição, sem valor a mercado" : "Valor a mercado das posições abertas");
}
// Coluna "Risco": exposição por casa, maior posição e drawdown, uma linha cada.
function pintarDestaquesVisao(d) {
  const el = document.getElementById("dashboardInsights");
  if (!el) return;
  const casas = Object.entries(d.casasAbertas).filter(([, g]) => g.stake > 0).sort((a, b) => b[1].stake - a[1].stake);
  const barra = casas.map(([, g], i) => `<i class="o${Math.min(i, 3)}" style="width:${(g.stake / d.stakeAbertas * 100).toFixed(2)}%"></i>`).join("");
  const legenda = casas.map(([casa, g], i) => `<span class="leg"><span class="key o${Math.min(i, 3)}"></span><img src="/assets/platforms/${casa}.png" alt="${escapeHtml(NOMES_CASAS[casa] || casa)}">${Math.round(g.stake / d.stakeAbertas * 100)}%</span>`).join("");
  const n = d.abertosF.length;
  const itens = [`<div class="visao-linha"><div class="grow">
<div class="kpi-abertas">${n ? `${n} ${n === 1 ? "posição aberta" : "posições abertas"}` : "Nenhuma posição aberta"}<button type="button" class="link" data-on-click="irPara('posicoes')">Ver posições</button></div>
${casas.length ? `<div class="pbar">${barra}</div><div class="kpi-legenda">${legenda}</div>` : ""}
</div></div>`];
  if (d.concMercado) {
    const pct = d.stakeAbertas > 0 ? d.concMercado.stake / d.stakeAbertas * 100 : 0;
    itens.push(`<button type="button" class="visao-linha" data-on-click="irPara('posicoes')"><div><div class="el destaque-titulo">${escapeHtml(d.concMercado.titulo)}</div><div class="sm mu">Maior exposição atual · ${pct.toFixed(1)}% do total</div></div><span class="val">${fmtDolar(d.concMercado.stake)}</span></button>`);
  }
  if (Number.isFinite(d.drawdownAtual)) {
    itens.push(`<div class="visao-linha"><div><div>Drawdown diário</div>${Number.isFinite(d.maxDrawdown) ? `<div class="sm mu">Máximo no histórico <b class="neg">${fmtDolar(d.maxDrawdown)}</b></div>` : ""}</div><span class="val ${d.drawdownAtual < 0 ? "neg" : ""}">${fmtDolar(d.drawdownAtual)}</span></div>`);
  }
  el.innerHTML = itens.join("");
}
// Séries diárias dos últimos 30 dias (todas as casas e cada casa) para os mini gráficos.
let seriesVisao = { chave: "", dados: new Map(), pendente: null };
function carregarSeriesVisao() {
  if (!indiceHistoricoLegivel()) return Promise.resolve(false);
  const casas = casasConectadas();
  const chave = [versaoLeituraHistorico(), ymdBrasil(new Date()), chavePeriodoFechadasConfigurado(), casas.join(",")].join("|");
  if (seriesVisao.chave === chave) return Promise.resolve(true);
  if (seriesVisao.pendente?.chave === chave) return seriesVisao.pendente.promise;
  const buscar = casa => fetch(urlLeituraHistorico("/api/history/closed/pnl?" + new URLSearchParams({ mode: "day", period: "30", platform: casa, now: String(Date.now()) })), { cache: "no-store" })
    .then(r => r.ok ? r.json() : null)
    .then(j => [casa, Array.isArray(j?.serie) ? j.serie.map(p => Number(p.v) || 0) : []])
    .catch(() => [casa, []]);
  const promise = Promise.all(["todas", ...casas].map(buscar)).then(lista => {
    if (seriesVisao.pendente?.promise !== promise) return false;
    seriesVisao = { chave, dados: new Map(lista), pendente: null };
    return true;
  });
  seriesVisao.pendente = { chave, promise };
  return promise;
}
function pintarMiniGraficosVisao() {
  for (const svg of document.querySelectorAll("svg.visao-spark[data-spark]")) {
    const diarios = seriesVisao.dados.get(svg.dataset.spark) || [];
    if (!diarios.length) { svg.innerHTML = ""; continue; }
    let acc = 0;
    const pontos = diarios.map(v => (acc += v));
    const min = Math.min(0, ...pontos), max = Math.max(0, ...pontos), faixa = max - min || 1;
    const passo = 120 / Math.max(1, pontos.length - 1);
    const linha = pontos.map((v, i) => `${(i * passo).toFixed(1)},${(24 - (v - min) / faixa * 22).toFixed(1)}`).join(" ");
    svg.setAttribute("class", "visao-spark " + (acc >= 0 ? "pos" : "neg"));
    svg.innerHTML = `<polyline points="${linha}" fill="none" stroke="currentColor" stroke-width="1.6" vector-effect="non-scaling-stroke"/>`;
  }
  const barras = document.getElementById("visaoBarras");
  if (barras) {
    const dias = (seriesVisao.dados.get("todas") || []).slice(-14);
    const maior = Math.max(0, ...dias.map(Math.abs)) || 1;
    barras.innerHTML = dias.map((v, i) => {
      const h = Math.max(v ? 2 : 0, Math.abs(v) / maior * 26);
      const x = 1 + i * 16;
      return v >= 0
        ? `<rect class="pos" x="${x}" y="${(30 - h).toFixed(1)}" width="10" height="${h.toFixed(1)}" rx="2"/>`
        : `<rect class="neg" x="${x}" y="30" width="10" height="${h.toFixed(1)}" rx="2"/>`;
    }).join("") + `<line x1="0" y1="30" x2="222" y2="30" class="base"/>`;
  }
}

// ===== Abertas agrupadas por casa (layout v2) =====
// Sem ordenação manual, as abertas ficam agrupadas por casa (na ordem do
// menu), mantendo a ordem por data dentro de cada grupo. O cabeçalho de cada
// grupo traz o total da casa no filtro atual, não só da página.
let gruposAbertasAtuais = null;
function agruparAbertasPorCasa(lista) {
  gruposAbertasAtuais = null;
  if (abaAtual !== "aberto" || sortCol) return;
  const ordem = new Map(CASAS_STAKE.map((casa, i) => [casa, i]));
  const posicao = new Map(lista.map((d, i) => [d, i]));
  lista.sort((a, b) => (ordem.get(casaDaAposta(a)) ?? 99) - (ordem.get(casaDaAposta(b)) ?? 99) || posicao.get(a) - posicao.get(b));
  gruposAbertasAtuais = new Map();
  for (const d of lista) {
    const casa = casaDaAposta(d);
    const g = gruposAbertasAtuais.get(casa) || { n: 0, stake: 0, pnl: 0, semCotacao: 0 };
    g.n += 1;
    g.stake += getStake(d);
    if (d._platform === "sxbet" && d.cashPnl == null) g.semCotacao += 1;
    else g.pnl += getPnl(d);
    gruposAbertasAtuais.set(casa, g);
  }
}
function linhaGrupoCasa(d, casaAnterior, cols) {
  if (!gruposAbertasAtuais || abaAtual !== "aberto") return "";
  const casa = casaDaAposta(d);
  if (casa === casaAnterior) return "";
  const g = gruposAbertasAtuais.get(casa);
  if (!g) return "";
  const nome = NOMES_CASAS[casa] || casa;
  const semValor = g.semCotacao === g.n;
  const pnl = semValor ? "—" : `${g.pnl >= 0 ? "+" : "-"}$${Math.abs(g.pnl).toFixed(2)}`;
  const classe = semValor ? "" : g.pnl > 0 ? "pnl-positive" : g.pnl < 0 ? "pnl-negative" : "";
  return `<tr class="grupo-casa"><td colspan="${cols}"><div class="grupo-casa-linha"><img class="casa-logo" src="/assets/platforms/${casa}.png" alt=""><strong>${escapeHtml(nome)}</strong><span class="grupo-casa-meta">${g.n} ${g.n === 1 ? "aberta" : "abertas"}</span><span class="grupo-casa-tot">Stake <b>$${g.stake.toFixed(2)}</b> · PnL <b class="${classe}">${pnl}</b></span></div></td></tr>`;
}

// Estado vazio das tabelas: limpa a busca e todos os filtros de uma vez.
function limparBuscaEFiltros() {
  const busca = document.getElementById("filtroTitulo");
  if (busca) busca.value = "";
  resetarFiltrosTabela({ renderizar: true });
}
