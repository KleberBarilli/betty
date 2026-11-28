// Oráculo do `classificarAposta()` para o porte no Rust (`categorias.rs`).
// Roda a função real da interface sobre as apostas do ledger, como a
// interface as recebe (`GET /api/history/closed`, com `_kalshiTitleMeta`),
// mais variantes e casos de borda, e grava a resposta do JS de cada caso.
//
//   node oraculo_categorias.mjs <pasta ui> <despejo .jsonl com GET /api/history/closed> <saída .jsonl>
//
// Depois, no Rust: BETTY_ORACULO_CATEGORIAS=<saída> cargo test --lib -- --ignored oraculo_categorias
// A saída tem dados reais da conta: apague no fim.
import vm from 'node:vm';
import fs from 'node:fs';
import path from 'node:path';
import readline from 'node:readline';

const [ui, despejo, saida] = process.argv.slice(2);
const SCRIPTS = ['i18n.js', 'i18n-en.js', 'audit-utils.js', 'market-title-utils.js', 'market-breakdown.js', 'odds-ranges.js', 'profit-calendar.js', 'acoes.js', 'dashboard.js'];
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
const armazenamento = new Map();
const g = {
  console: { log() {}, warn() {}, error() {}, info() {}, debug() {} }, URL, URLSearchParams, TextEncoder, TextDecoder, structuredClone, queueMicrotask, performance,
  setTimeout: () => 0, clearTimeout() {}, setInterval: () => 0, clearInterval() {}, requestAnimationFrame: () => 0, requestIdleCallback: () => 0, cancelIdleCallback() {},
  fetch: () => Promise.reject(new Error('sem rede')),
  localStorage: { getItem: k => armazenamento.get(k) ?? null, setItem: (k, v) => armazenamento.set(k, String(v)), removeItem: k => armazenamento.delete(k) },
  location: { origin: 'http://betty.localhost', href: '' }, document: stub(), navigator: stub(), matchMedia: stub(), getComputedStyle: stub(), CSS: stub(),
  MutationObserver: function () { return stub(); }, ResizeObserver: function () { return stub(); }, HTMLElement: function () {}, Element: function () {}, Event: function () {},
  addEventListener() {},
};
const ctx = vm.createContext(g);
vm.runInContext('globalThis.window = globalThis;', ctx);
for (const s of SCRIPTS) vm.runInContext(fs.readFileSync(path.join(ui, s), 'utf8'), ctx, { filename: s });
// Cada caso vai como texto e volta como texto: objetos entre realms não se comparam.
vm.runInContext(`globalThis.__classificar = texto => {
  try { return JSON.stringify({ saida: classificarAposta(JSON.parse(texto)) }); }
  catch (e) { return JSON.stringify({ erro: String(e?.message || e) }); }
};`, ctx);

async function apostasDoDespejo() {
  const linhas = readline.createInterface({ input: fs.createReadStream(despejo, 'utf8'), crlfDelay: Infinity });
  for await (const linha of linhas) {
    if (!linha.startsWith('{"url":"/api/history/closed"')) continue;
    return JSON.parse(linha).body.fechados;
  }
  throw new Error('o despejo não tem GET /api/history/closed');
}
const apostas = await apostasDoDespejo();
const n = apostas.length;
const casos = [];
const juntar = (id, entrada) => casos.push({ id, entrada });
apostas.forEach((a, i) => {
  const outra = apostas[(i + 7) % n], terceira = apostas[(i + 13) % n];
  juntar(`ledger:${i}`, a);
  const { _sportMeta, ...semMeta } = a;
  juntar(`sem-meta:${i}`, semMeta);
  juntar(`titulo-de-outra:${i}`, { ...a, title: outra.title });
  juntar(`outra-casa:${i}`, { ...a, _platform: a._platform === 'kalshi' ? 'polymarket' : 'kalshi' });
  const { sport, category, sportName, sportsCategory, ...semEsporte } = a;
  juntar(`sem-esporte:${i}`, semEsporte);
  juntar(`slugs-de-outra:${i}`, { ...a, slug: terceira.slug, eventSlug: terceira.eventSlug, _eventTicker: terceira._eventTicker });
  if (_sportMeta && typeof _sportMeta === 'object') juntar(`meta-de-outra:${i}`, { ...a, _sportMeta: outra._sportMeta, asset: outra.asset, conditionId: outra.conditionId });
});
// Casos de borda: exceções, tipos estranhos e espaços que o JS trata diferente do Rust.
const borda = [
  null, 'texto', 5, [], [1, 2],
  { _platform: 'polymarket', _sportMeta: { markets: [null] } },
  { _platform: 'polymarket', _sportMeta: { markets: 'abc' } },
  { _platform: 'polymarket', _sportMeta: { markets: [5, 'x', [], {}] } },
  { _platform: 'polymarket', _sportMeta: { sportsMarketTypes: 5 } },
  { _platform: 'polymarket', _sportMeta: { sportsMarketTypes: 'tennis_x' } },
  { _platform: 'polymarket', _sportMeta: { sportsMarketTypes: {} } },
  { _platform: 'polymarket', _sportMeta: { sportsMarketTypes: ['soccer_moneyline', null, 0] } },
  { _platform: 'polymarket', _sportMeta: 'texto', title: 'Lakers vs Celtics' },
  { _platform: 'polymarket', _sportMeta: { texts: ['NBA', 'lakers-celtics'], markets: [{ slug: 'lakers-celtics' }] }, title: 'x' },
  { _platform: 'polymarket', _sportMeta: { eventTexts: 'não é lista', texts: ['NHL'] }, title: 'x' },
  { _platform: 'polymarket', title: '﻿soccer\u0085', sport: '﻿' },
  { _platform: 'polymarket', title: 'Arsenal vs Chelsea' },
  { _platform: 'polymarket', title: 'Will Taylor perform\r"Love Story"?' },
  { _platform: 'polymarket', title: 'Will they sing "x"?' },
  { _platform: 'polymarket', title: 'İSTANBUL BAŞAKŞEHİR vs Galatasaray', sport: 'ΣΟΚΚΕΡ' },
  { _platform: 'polymarket', title: 'Straße vs Weg', slug: 'ß-ss' },
  { _platform: 'polymarket', title: 'tênis atp' },
  { _platform: 'polymarket', title: 'A vs B', sport: ['Soccer', 'x'] },
  { _platform: 'polymarket', title: 'A vs B', sport: { nome: 'Soccer' } },
  { _platform: 'polymarket', title: 0, slug: 0, sport: 0, category: false },
  { _platform: 'KALSHI', title: 'X', _kalshiTitleMeta: { market: { rules_primary: 'in the Lakers vs Celtics basketball game originally' } } },
  { _platform: 'kalshi', _kalshiTitleMeta: { market: null } },
  { _platform: 'predict', _category: 'Política' }, { _platform: 'sxbet', _category: 7 }, { _platform: 'sxbet', _category: '' }, { _platform: 'predict' },
  { _platform: 'polymarket', title: 'Super Bowl LX: how many viewers?', sport: 'NFL' },
  { _platform: 'polymarket', title: 'Will Neymar cry at the World Cup?' },
  { _platform: 'polymarket', title: 'Will OpenAI release GPT-6?', slug: 'epl-ars-che-2026-01-12-x' },
  { _platform: 'kalshi', title: 'Over 5.5', _eventTicker: 'KXKBOTOTAL-26SEP' },
  { _platform: 'polymarket', title: 'x', slug: 'bkarg-boca-river-2026' },
];
borda.forEach((b, i) => juntar(`borda:${i}`, b));

const fd = fs.openSync(saida, 'w');
let erros = 0;
for (const caso of casos) {
  const texto = JSON.stringify(caso.entrada);
  const r = JSON.parse(vm.runInContext(`__classificar(${JSON.stringify(texto)})`, ctx));
  if (r.erro) erros++;
  fs.writeSync(fd, JSON.stringify({ id: caso.id, entrada: caso.entrada, ...r }) + '\n');
}
fs.closeSync(fd);
console.log(`${casos.length} casos (${n} apostas do ledger, ${borda.length} de borda), ${erros} com exceção no JS`);
