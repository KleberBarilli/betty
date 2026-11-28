// Oráculo da compactação do _sportMeta: roda as funções reais da interface
// (dashboard.js e companhia) sobre cada aposta do banco original e do banco
// compactado e compara tudo o que a interface deriva delas.
// uso: node oraculo_compactacao.mjs <orig.db> <comp.db> <pasta ui>
import { DatabaseSync } from 'node:sqlite';
import vm from 'node:vm';
import fs from 'node:fs';
import path from 'node:path';

const [orig, comp, ui] = process.argv.slice(2);
const SCRIPTS = ['i18n.js', 'i18n-en.js', 'audit-utils.js', 'market-title-utils.js', 'market-breakdown.js', 'odds-ranges.js', 'profit-calendar.js', 'acoes.js', 'dashboard.js'];

function stub() {
  const f = function () {};
  const p = new Proxy(f, {
    get(_, k) {
      if (k === Symbol.toPrimitive) return () => '';
      if (k === Symbol.iterator) return function* () {};
      if (k === 'then') return undefined;
      if (k === 'length') return 0;
      if (k === 'toString' || k === 'valueOf') return () => '';
      return p;
    },
    apply() { return p; },
    construct() { return p; },
    set() { return true; },
  });
  return p;
}

function contexto() {
  const armazenamento = new Map();
  const g = {
    console: { log() {}, warn() {}, error() {}, info() {}, debug() {} },
    URL, URLSearchParams, TextEncoder, TextDecoder, structuredClone, queueMicrotask, performance,
    setTimeout: () => 0, clearTimeout() {}, setInterval: () => 0, clearInterval() {},
    requestAnimationFrame: () => 0, cancelAnimationFrame() {}, requestIdleCallback: () => 0, cancelIdleCallback() {},
    fetch: () => Promise.reject(new Error('sem rede no oráculo')),
    localStorage: {
      getItem: k => (armazenamento.has(k) ? armazenamento.get(k) : null),
      setItem: (k, v) => armazenamento.set(k, String(v)),
      removeItem: k => armazenamento.delete(k),
      key: () => null, get length() { return armazenamento.size; }, clear: () => armazenamento.clear(),
    },
    location: { origin: 'http://betty.localhost', href: 'http://betty.localhost/dashboard.html', pathname: '/dashboard.html' },
    document: stub(), navigator: stub(), matchMedia: stub(), getComputedStyle: stub(), CSS: stub(),
    HTMLElement: function () {}, Element: function () {}, Node: function () {}, Event: function () {}, CustomEvent: function () {},
    MutationObserver: function () { return stub(); }, ResizeObserver: function () { return stub(); }, IntersectionObserver: function () { return stub(); },
    addEventListener() {}, removeEventListener() {}, dispatchEvent() {},
  };
  const ctx = vm.createContext(g);
  vm.runInContext('globalThis.window = globalThis; globalThis.self = globalThis;', ctx);
  for (const s of SCRIPTS) {
    vm.runInContext(fs.readFileSync(path.join(ui, s), 'utf8'), ctx, { filename: s });
  }
  vm.runInContext(`
    globalThis.__avaliar = (texto) => {
      const saida = {};
      const tentar = (nome, f) => {
        try { const v = f(JSON.parse(texto)); saida[nome] = v === undefined ? '__undefined' : v; }
        catch (e) { saida[nome] = { erro: String(e && e.message || e) }; }
      };
      tentar('indice', d => { const i = indiceHistoricoFechado(d); delete i.sourceSignature; return i; });
      tentar('indicePlatform', d => { const i = indiceHistoricoFechado({ ...d, _platform: d._platform || 'polymarket' }); delete i.sourceSignature; return i; });
      tentar('tituloExibicao', d => tituloExibicao(d));
      tentar('tituloCSV', d => tituloExibicaoCSV(d));
      tentar('categoria', d => classificarAposta(d));
      tentar('nomeMercado', d => normalizarNomeMercadoPolymarket(d));
      tentar('exactMarket', d => globalThis.BettyMarketTitles.exactMarket(d));
      tentar('combate', d => globalThis.BettyMarketTitles.combatTitle(d));
      tentar('incompleta', d => fechadaPolymarketIncompleta(d));
      tentar('tiposMercado', d => esportePorTiposMercado(d));
      tentar('sinais', d => sinaisDaAposta(d));
      return JSON.stringify(saida);
    };`, ctx);
  return ctx;
}

const ler = arquivo => {
  const db = new DatabaseSync(arquivo, { readOnly: true });
  const linhas = db.prepare('SELECT platform, bet_key, data_json, updated_at FROM closed_bets_ledger ORDER BY rowid').all();
  db.close();
  return new Map(linhas.map(l => [l.platform + '|' + l.bet_key, l]));
};
const A = ler(orig), B = ler(comp);
const ctxA = contexto(), ctxB = contexto();
const avaliar = (ctx, texto) => { ctx.__t = texto; return vm.runInContext('__avaliar(__t)', ctx); };

let total = 0, compactadas = 0, iguais = 0, bytesA = 0, bytesB = 0;
const diferencas = [], porCampo = {};
if (A.size !== B.size) diferencas.push({ erro: `quantidade de apostas: ${A.size} x ${B.size}` });
for (const [chave, a] of A) {
  const b = B.get(chave);
  total++;
  if (!b) { diferencas.push({ chave, erro: 'sumiu no compactado' }); continue; }
  bytesA += Buffer.byteLength(a.data_json); bytesB += Buffer.byteLength(b.data_json);
  if (a.updated_at !== b.updated_at) diferencas.push({ chave, erro: 'updated_at mudou' });
  if (a.data_json === b.data_json) { iguais++; continue; }
  compactadas++;
  if (a.platform !== 'polymarket') { diferencas.push({ chave, erro: 'aposta fora da Polymarket mudou' }); continue; }
  // Fora do _sportMeta, o texto tem que ser o mesmo.
  const semMeta = t => { const o = JSON.parse(t); delete o._sportMeta; return JSON.stringify(o); };
  if (semMeta(a.data_json) !== semMeta(b.data_json)) { diferencas.push({ chave, erro: 'campos fora do _sportMeta mudaram' }); continue; }
  const ra = JSON.parse(avaliar(ctxA, a.data_json)), rb = JSON.parse(avaliar(ctxB, b.data_json));
  for (const campo of Object.keys(ra)) {
    const [x, y] = [JSON.stringify(ra[campo]), JSON.stringify(rb[campo])];
    if (x !== y) {
      porCampo[campo] = (porCampo[campo] || 0) + 1;
      if (diferencas.length < 20) diferencas.push({ chave, campo, original: ra[campo], compactado: rb[campo] });
    }
  }
}
// Erros que o JS lançou no original (devem aparecer iguais no compactado).
let errosOriginal = 0;
for (const [, a] of A) {
  if (a.platform !== 'polymarket') continue;
  const r = JSON.parse(avaliar(ctxA, a.data_json));
  if (Object.values(r).some(v => v && typeof v === 'object' && 'erro' in v && Object.keys(v).length === 1)) errosOriginal++;
}
console.log(JSON.stringify({ total, inalteradas: iguais, compactadas, mbAntes: +(bytesA / 1048576).toFixed(1), mbDepois: +(bytesB / 1048576).toFixed(1), errosOriginal, porCampo, diferencas }, null, 1));
process.exit(diferencas.length ? 1 : 0);
