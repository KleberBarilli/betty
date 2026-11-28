// Abertura com dados salvos: dashboard.js real, DOM falso e backend falso.
import vm from 'node:vm';
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';

const ui = process.argv[2];
function stub() {
  const p = new Proxy(function () {}, {
    get(_, k) {
      if (k === Symbol.toPrimitive) return () => '';
      if (k === Symbol.iterator) return function* () {};
      if (k === 'then') return undefined;
      if (k === 'length') return 0;
      return p;
    },
    apply() { return p; }, construct() { return p; }, set() { return true; },
  });
  return p;
}
function elemento() {
  const classes = new Set();
  return {
    hidden: false, textContent: '', title: '', innerHTML: '', value: '', dataset: {}, style: { setProperty() {} },
    classList: { toggle(c, v) { (v ?? !classes.has(c)) ? classes.add(c) : classes.delete(c); }, add: c => classes.add(c), remove: c => classes.delete(c), contains: c => classes.has(c) },
    classes, querySelector: () => stub(), querySelectorAll: () => [], setAttribute() {}, getAttribute: () => null, append() {}, replaceChildren() {},
    insertAdjacentHTML() {}, addEventListener() {}, closest: () => null, hasChildNodes: () => false,
  };
}

function cenario({ salvo, falhaSync }) {
  const log = [];
  const estado = { indexados: salvo ? 3 : 0 };
  const elementos = new Map(['seloDadosSalvos', 'seloDadosSalvosTexto', 'syncProgress'].map(id => [id, elemento()]));
  elementos.get('seloDadosSalvos').hidden = true; // como no HTML
  const armazenamento = new Map();
  if (salvo) {
    armazenamento.set('betty_pos_cache_v3', JSON.stringify({ t: Date.UTC(2026, 10, 27, 15, 20), abertos: [{ _platform: 'polymarket', conditionId: '0x1', asset: '1', outcome: 'Yes', title: 'A vs B', size: 2, initialValue: 1, currentValue: 1.5 }], fechados: [] }));
    armazenamento.set('betty:saldosSalvos:v1', JSON.stringify({ polymarket: { v: 9.9, t: Date.UTC(2026, 10, 27, 15, 20) } }));
  }
  const ouvintes = {};
  const atrasada = (status, corpo) => new Promise(r => setTimeout(() => r(resposta(status, corpo)), 150));
  const resposta = (status, corpo) => Promise.resolve({ ok: status < 400, status, headers: { get: () => null }, json: async () => corpo, text: async () => JSON.stringify(corpo), body: { cancel() {} } });
  const fetch = (url, opts = {}) => {
    const metodo = (opts.method || 'GET').toUpperCase();
    const u = new URL(url, 'http://betty.localhost');
    log.push(metodo + ' ' + u.pathname + (u.searchParams.get('snapshot') ? ' [salvo]' : ''));
    const r = u.pathname;
    if (r === '/api/me') return resposta(200, { user: { id: 'u1', name: 'Usuário' }, connections: { polymarket: true } });
    if (r === '/api/settings') return resposta(200, { ok: true });
    if (r === '/api/history/snapshot') return resposta(200, { ok: true });
    if (r === '/api/history/closed/index/status') return resposta(200, { ok: true, ledger: estado.indexados, indexed: estado.indexados, pending: 0, complete: true, versions: {} });
    if (r === '/api/history/closed/index/versions') return resposta(200, { ok: true, versions: JSON.parse(opts.body).versions });
    if (r.startsWith('/api/history/closed/stats')) return resposta(200, { ok: true, total: { total: estado.indexados, lucroD: 5 } });
    if (r === '/api/history/closed/pnl') return resposta(200, { ok: true, serie: [], missingDates: 0 });
    if (r === '/api/history/closed/calendar') return resposta(200, { ok: true, days: [] });
    if (r === '/api/history/closed/query') return resposta(200, { ok: true, total: 0, fechados: [] });
    if (r === '/api/polymarket/balance') return falhaSync ? atrasada(502, { ok: false }) : atrasada(200, { ok: true, balance: 12.5 });
    if (r.startsWith('/api/polymarket/')) return falhaSync ? atrasada(500, { ok: false }) : atrasada(200, []);
    if (r === '/api/history/closed/lookup') return resposta(200, { ok: true, entries: [] });
    if (r === '/api/history/closed/delta') return resposta(200, { ok: true, fechados: [] });
    if (r === '/api/history/closed/index') return resposta(200, { ok: true });
    if (r === '/api/history/closed/recover-pnl-dates') return resposta(200, { ok: true });
    if (r === '/api/history/closed') return resposta(200, { ok: true, fechados: [] });
    if (r === '/api/history/closed/metadata' || r === '/api/polymarket-metadata/cache') return resposta(200, { ok: true, entries: [] });
    return resposta(404, { ok: false });
  };
  const g = {
    console: { log() {}, warn() {}, error() {}, info() {}, debug() {} }, URL, URLSearchParams, TextEncoder, TextDecoder, structuredClone, queueMicrotask,
    performance: { now: () => performance.now(), measure() {}, getEntriesByType: () => [], clearMeasures() {} },
    setTimeout: (f, ms, ...a) => { const t = setTimeout(f, Math.min(ms || 0, 20), ...a); t.unref?.(); return t; }, clearTimeout,
    setInterval: () => 0, clearInterval() {}, requestAnimationFrame: () => 0, cancelAnimationFrame() {}, requestIdleCallback: () => 0, cancelIdleCallback() {},
    fetch,
    localStorage: { getItem: k => (armazenamento.has(k) ? armazenamento.get(k) : null), setItem: (k, v) => armazenamento.set(k, String(v)), removeItem: k => armazenamento.delete(k) },
    location: { origin: 'http://betty.localhost', href: 'http://betty.localhost/dashboard.html' },
    document: new Proxy(stub(), {
      get(alvo, k) {
        if (k === 'getElementById') return id => elementos.get(id) || stub();
        if (k === 'addEventListener') return (tipo, f) => { ouvintes[tipo] = f; };
        if (k === 'hidden') return false;
        return alvo[k];
      },
    }),
    navigator: stub(), matchMedia: () => ({ matches: true, addEventListener() {} }), getComputedStyle: stub(), CSS: stub(),
    MutationObserver: function () { return stub(); }, ResizeObserver: function () { return stub(); }, HTMLElement: function () {}, Element: function () {}, Event: function () {},
    addEventListener() {}, confirm: () => true,
  };
  const ctx = vm.createContext(g);
  vm.runInContext('globalThis.window = globalThis;', ctx);
  for (const s of ['i18n.js', 'i18n-en.js', 'audit-utils.js', 'market-title-utils.js', 'market-breakdown.js', 'odds-ranges.js', 'profit-calendar.js', 'acoes.js', 'dashboard.js']) {
    vm.runInContext(fs.readFileSync(path.join(ui, s), 'utf8'), ctx, { filename: s });
  }
  const ler = expr => JSON.parse(vm.runInContext(`JSON.stringify(${expr})`, ctx));
  return { log, elementos, armazenamento, ouvintes, ler, estado };
}
const esperar = async (cond, ms = 4000) => { const fim = Date.now() + ms; while (!cond()) { if (Date.now() > fim) throw new Error('tempo esgotado'); await new Promise(r => setTimeout(r, 10)); } };

// A. Dados salvos + sincronização que dá certo.
{
  const c = cenario({ salvo: true, falhaSync: false });
  const abertura = c.ouvintes.DOMContentLoaded();
  await esperar(() => c.log.includes('POST /api/history/snapshot'));
  await esperar(() => c.ler('exibindoDadosSalvos'));
  assert.equal(c.ler('POLY_API_BALANCE'), 9.9, 'saldo salvo aparece na abertura');
  assert.match(c.elementos.get('seloDadosSalvosTexto').textContent, /^Atualizando · dados de /);
  assert.equal(c.elementos.get('seloDadosSalvos').hidden, false);
  await abertura;
  await esperar(() => c.log.includes('DELETE /api/history/snapshot'));
  const iSalvo = c.log.indexOf('POST /api/history/snapshot');
  const primeiraEscrita = c.log.findIndex(l => /POST \/api\/(history\/closed\/(delta|index$)|tracked)/.test(l) || l.startsWith('GET /api/polymarket/'));
  assert.ok(iSalvo < primeiraEscrita, 'o estado salvo é congelado antes de a sincronização começar');
  const leiturasAntes = c.log.slice(iSalvo, c.log.indexOf('DELETE /api/history/snapshot')).filter(l => /\/(stats|pnl|calendar|query)/.test(l));
  assert.ok(leiturasAntes.length > 0 && leiturasAntes.every(l => l.endsWith('[salvo]')), 'leituras durante os dados salvos usam o estado salvo: ' + leiturasAntes);
  assert.equal(c.ler('exibindoDadosSalvos'), false);
  assert.equal(c.ler('POLY_API_BALANCE'), 12.5, 'saldo confirmado substitui o salvo');
  assert.equal(c.elementos.get('seloDadosSalvos').hidden, true, 'selo some ao confirmar');
  assert.equal(JSON.parse(c.armazenamento.get('betty:saldosSalvos:v1')).polymarket.v, 12.5, 'saldo confirmado fica salvo para a próxima abertura');
  await new Promise(r => setTimeout(r, 50));
  const depois = c.log.slice(c.log.indexOf('DELETE /api/history/snapshot')).filter(l => /\/(stats|pnl|calendar|query)/.test(l));
  assert.ok(depois.every(l => !l.endsWith('[salvo]')), 'depois de confirmar, as leituras voltam ao banco principal');
  console.log('A ok:', c.log.length, 'chamadas; leituras no estado salvo:', leiturasAntes.length);
}
// B. Dados salvos + sincronização que falha.
{
  const c = cenario({ salvo: true, falhaSync: true });
  await c.ouvintes.DOMContentLoaded();
  await esperar(() => c.elementos.get('seloDadosSalvos').classes.has('aviso'));
  assert.equal(c.ler('exibindoDadosSalvos'), true, 'continua nos dados salvos');
  assert.match(c.elementos.get('seloDadosSalvosTexto').textContent, /não foi possível atualizar/);
  assert.equal(c.ler('POLY_API_BALANCE'), 9.9, 'saldo salvo continua visível');
  assert.ok(!c.log.includes('DELETE /api/history/snapshot'));
  assert.equal(JSON.parse(c.armazenamento.get('betty:saldosSalvos:v1')).polymarket.t, Date.UTC(2026, 10, 27, 15, 20), 'saldo não reconfirmado mantém a data antiga');
  console.log('B ok:', c.elementos.get('seloDadosSalvosTexto').textContent);
}
// C. Primeira abertura, sem nada salvo: tela de carregamento como antes.
{
  const c = cenario({ salvo: false, falhaSync: false });
  await c.ouvintes.DOMContentLoaded();
  assert.ok(!c.log.includes('POST /api/history/snapshot'), 'sem dados salvos não congela nada');
  assert.equal(c.ler('exibindoDadosSalvos'), false);
  assert.equal(c.elementos.get('seloDadosSalvos').hidden, true, 'selo nunca aparece');
  await esperar(() => c.log.includes('GET /api/history/closed/stats'));
  assert.ok(!c.log.some(l => l.endsWith('[salvo]')), 'leituras normais, sem estado salvo');
  console.log('C ok');
}
console.log('abertura com dados salvos: 3 cenários ok');
