// Fluxo das versões do índice no dashboard.js real, com fetch simulado.
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
      return p;
    },
    apply() { return p; }, construct() { return p; }, set() { return true; },
  });
  return p;
}
function contexto({ banco, local = {}, falhaBanco = false }) {
  const armazenamento = new Map(Object.entries(local));
  const posts = [];
  const resposta = (ok, corpo) => Promise.resolve({ ok, status: ok ? 200 : 500, json: async () => corpo });
  const g = {
    console: { log() {}, warn() {}, error() {}, info() {} }, URL, URLSearchParams, TextEncoder, TextDecoder, performance,
    setTimeout: () => 0, clearTimeout() {}, setInterval: () => 0, clearInterval() {}, requestAnimationFrame: () => 0,
    localStorage: {
      getItem: k => (armazenamento.has(k) ? armazenamento.get(k) : null), setItem: (k, v) => armazenamento.set(k, String(v)),
      removeItem: k => armazenamento.delete(k),
    },
    location: { origin: 'http://betty.localhost', href: '' }, document: stub(), navigator: stub(), matchMedia: stub(),
    addEventListener() {}, MutationObserver: function () { return stub(); }, HTMLElement: function () {}, Element: function () {}, Event: function () {},
    fetch: (url, opts = {}) => {
      if (url === '/api/history/closed/index/versions') {
        const corpo = JSON.parse(opts.body);
        posts.push(corpo.versions);
        if (falhaBanco) return resposta(false, null);
        Object.assign(banco, corpo.versions);
        return resposta(true, { ok: true, versions: { ...banco } });
      }
      return Promise.reject(new Error('sem rede'));
    },
  };
  const ctx = vm.createContext(g);
  vm.runInContext('globalThis.window = globalThis;', ctx);
  for (const s of ['i18n.js', 'i18n-en.js', 'audit-utils.js', 'market-title-utils.js', 'market-breakdown.js', 'odds-ranges.js', 'profit-calendar.js', 'acoes.js', 'dashboard.js']) {
    vm.runInContext(fs.readFileSync(path.join(ui, s), 'utf8'), ctx, { filename: s });
  }
  return { ctx, armazenamento, posts, run: code => vm.runInContext(code, ctx) };
}
const atuais = c => JSON.parse(c.run('JSON.stringify({category: VERSAO_INDICE_CATEGORIA, title: VERSAO_INDICE_TITULO, date: VERSAO_INDICE_DATA})'));
const precisa = c => JSON.parse(c.run('JSON.stringify([categoriaIndicePrecisaReindexar(), tituloIndicePrecisaReindexar(), dataIndicePrecisaReindexar()])'));

// 1. Quem já tinha o índice montado (marca no navegador) leva a marca ao banco, sem reindexar.
{
  const banco = {};
  const c0 = contexto({ banco });
  const v = atuais(c0);
  const local = { 'betty:categoryIndexVersion:local': v.category, 'betty:dateIndexVersion:local': 'velha' };
  const c = contexto({ banco, local });
  c.run('versoesIndiceHistoricoDb = {}');
  await c.run('trazerVersoesIndiceDoNavegador()');
  assert.deepEqual(c.posts, [{ category: v.category }], 'só a versão atual do navegador vai ao banco');
  assert.deepEqual(precisa(c), [false, true, true]);
  assert.deepEqual(banco, { category: v.category });
}
// 2. Banco com as versões atuais: nada a refazer, mesmo com o navegador vazio (perfil novo).
{
  const c0 = contexto({ banco: {} });
  const banco = { ...atuais(c0) };
  const c = contexto({ banco });
  c.run(`versoesIndiceHistoricoDb = ${JSON.stringify(banco)}`);
  await c.run('trazerVersoesIndiceDoNavegador()');
  assert.deepEqual(c.posts, []);
  assert.deepEqual(precisa(c), [false, false, false]);
}
// 3. Sem o status do banco: vale o navegador, como antes.
{
  const c0 = contexto({ banco: {} });
  const v = atuais(c0);
  const c = contexto({ banco: {}, local: { 'betty:titleIndexVersion:local': v.title } });
  c.run('versoesIndiceHistoricoDb = null');
  await c.run('trazerVersoesIndiceDoNavegador()');
  assert.deepEqual(c.posts, []);
  assert.deepEqual(precisa(c), [true, false, true]);
}
// 4. Registrar depois de reindexar grava no banco; se o banco falhar, cai no navegador.
{
  const banco = {};
  const c = contexto({ banco });
  c.run('versoesIndiceHistoricoDb = {}');
  await c.run('registrarVersoesIndiceHistorico(["category", "date"])');
  assert.deepEqual(Object.keys(banco).sort(), ['category', 'date']);
  assert.deepEqual(precisa(c), [false, true, false]);
  assert.equal(c.armazenamento.size, 0, 'com o banco, o navegador não é usado');
  const f = contexto({ banco: {}, falhaBanco: true });
  f.run('versoesIndiceHistoricoDb = {}');
  await f.run('registrarVersoesIndiceHistorico(["title"])');
  assert.equal(f.armazenamento.get('betty:titleIndexVersion:local'), atuais(f).title);
  assert.deepEqual(precisa(f), [true, false, true]);
}
// 5. Cache das fechadas: só em memória e apaga a cópia antiga do navegador.
{
  const c = contexto({ banco: {}, local: { 'betty:polyClosedEnriched:local': '{"entries":{"x":{}}}' } });
  const entradas = c.run('Object.keys(cacheFechadasPolymarket().entries).length');
  assert.equal(entradas, 0);
  assert.equal(c.armazenamento.has('betty:polyClosedEnriched:local'), false);
  assert.equal(c.run('typeof salvarCacheFechadasPolymarket'), 'undefined');
}
console.log('versões do índice e cache das fechadas: 5 cenários ok');
