// Oráculo do recorte do cache de metadados por evento: as funções reais da
// interface com o evento inteiro e com o evento recortado devem dar o mesmo
// resultado para cada aposta que pediu e para a data do PnL das apostas do
// ledger. uso: node oraculo_recorte.mjs <pasta com respostas> <pasta ui>
import vm from 'node:vm';
import fs from 'node:fs';
import path from 'node:path';

const [pasta, ui] = process.argv.slice(2);
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
function contexto() {
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
  vm.runInContext(`
    globalThis.__mapas = null;
    globalThis.__usarMapas = texto => { __mapas = JSON.parse(texto); metasDatasPnl = __mapas.metas; };
    globalThis.__avaliar = (texto, base) => {
      const r = JSON.parse(texto), m = __mapas, s = {};
      const tentar = (nome, f) => { try { const v = f(); s[nome] = v === undefined ? '__u' : v; } catch (e) { s[nome] = { erro: String(e?.message || e) }; } };
      const meta = m.sport[r.eventSlug] || m.sport[r.slug];
      tentar('exato', () => globalThis.BettyMarketTitles.exactMarket(r, meta));
      tentar('temToken', () => Array.isArray(meta?.markets) && meta.markets.some(x => Array.isArray(x?.tokenIds) && x.tokenIds.length));
      const data = resolverDataInicio(r, m.start, m.metas);
      s.data = data;
      s.resolvido = r._resolvedTime || m.metas[r.eventSlug]?.markets?.[r.slug]?.closedTime || m.metas[r.slug]?.markets?.[r.slug]?.closedTime || null;
      const abertos = { ...r, _startDate: data.data, _dateSource: data.fonte, _dateSuspect: data.suspeita, _resolvedTime: s.resolvido, _sportMeta: meta || null };
      const fechada = { ...abertos, _platform: 'polymarket', _sportMeta: compactarMetaEsporteParaCache(meta) || null };
      for (const [rotulo, d] of [['aberta', abertos], ['fechada', fechada]]) {
        tentar(rotulo + ':titulo', () => tituloExibicao(d));
        tentar(rotulo + ':categoria', () => classificarAposta(d));
        tentar(rotulo + ':nome', () => normalizarNomeMercadoPolymarket(d));
        tentar(rotulo + ':incompleta', () => fechadaPolymarketIncompleta(d));
        tentar(rotulo + ':tipos', () => esportePorTiposMercado(d));
        tentar(rotulo + ':sinais', () => sinaisDaAposta(d));
        tentar(rotulo + ':indice', () => { const i = indiceHistoricoFechado(d); delete i.sourceSignature; return i; });
      }
      // Data do PnL das apostas do ledger do evento (sem _resolvedTime usa o mapa de datas).
      tentar('pnl', () => base.map(t => getTsResultadoPnL(JSON.parse(t))));
      return JSON.stringify(s);
    };`, ctx);
  return ctx;
}
const ler = f => fs.readFileSync(path.join(pasta, f), 'utf8').trim().split('\n').map(l => JSON.parse(l));
const respostas = ler('saida_meta_evento.jsonl');
const pedidos = JSON.parse(fs.readFileSync(path.join(pasta, 'pedidos.json'), 'utf8'));
const ledgerPorEvento = JSON.parse(fs.readFileSync(path.join(pasta, 'ledger_por_evento.json'), 'utf8'));
const mapas = { A: { cheio: {}, recorte: {} }, B: { cheio: {}, recorte: {} }, C: { cheio: {}, recorte: {} } };
let bytes = { cheio: 0, recorte: 0 }, ms = { cheio: 0, recorte: 0 };
for (const r of respostas) {
  const [, , arq] = r.url.split(' ');
  const [cen, tipo] = arq.split('_');
  bytes[tipo] += Buffer.byteLength(JSON.stringify(r.body)); ms[tipo] += r.ms;
  for (const e of r.body.entries || []) mapas[cen][tipo][e.slug] = e.data;
}
const ctxA = contexto(), ctxB = contexto();
const montar = m => JSON.stringify({
  start: Object.fromEntries(Object.entries(m).map(([k, v]) => [k, v.start])),
  metas: Object.fromEntries(Object.entries(m).map(([k, v]) => [k, v.dates])),
  sport: Object.fromEntries(Object.entries(m).map(([k, v]) => [k, v.sport])),
});
const relatorio = { bytesCheio: +(bytes.cheio / 1048576).toFixed(1), bytesRecorte: +(bytes.recorte / 1048576).toFixed(1), msCheio: Math.round(ms.cheio), msRecorte: Math.round(ms.recorte) };
const LIMPAR = ['_sportMeta', '_startDate', '_dateSource', '_dateSuspect'];
for (const cen of ['A', 'B', 'C']) {
  const cheio = mapas[cen].cheio, recorte = mapas[cen].recorte;
  vm.runInContext(`__usarMapas(${JSON.stringify(montar(cheio))})`, ctxA);
  vm.runInContext(`__usarMapas(${JSON.stringify(montar(recorte))})`, ctxB);
  let apostas = 0, iguais = 0, menores = 0, eventos = 0, pnlChecados = 0;
  const difs = {}, exemplos = [];
  for (const [ev, rows] of Object.entries(pedidos[cen])) {
    if (!cheio[ev]) continue;
    eventos++;
    if (JSON.stringify(recorte[ev]).length < JSON.stringify(cheio[ev]).length) menores++;
    const base = JSON.stringify((ledgerPorEvento[ev] || []).map(t => JSON.stringify(t)));
    pnlChecados += (ledgerPorEvento[ev] || []).length;
    for (const original of rows) {
      for (const variante of [0, 1]) {
        const r = { ...original };
        for (const k of LIMPAR) delete r[k];
        if (variante) delete r._resolvedTime;
        apostas++;
        ctxA.__t = JSON.stringify(r); ctxA.__b = JSON.parse(base);
        ctxB.__t = JSON.stringify(r); ctxB.__b = JSON.parse(base);
        const a = JSON.parse(vm.runInContext('__avaliar(__t, __b)', ctxA));
        const b = JSON.parse(vm.runInContext('__avaliar(__t, __b)', ctxB));
        let igual = true;
        for (const k of Object.keys(a)) {
          if (JSON.stringify(a[k]) !== JSON.stringify(b[k])) {
            igual = false; difs[k] = (difs[k] || 0) + 1;
            if (exemplos.length < 5) exemplos.push({ ev, k, cheio: a[k], recorte: b[k] });
          }
        }
        if (igual) iguais++;
      }
    }
  }
  relatorio[cen] = { eventos, eventosMenores: menores, avaliacoes: apostas, iguais, datasPnlDoLedger: pnlChecados, difs, exemplos };
}
console.log(JSON.stringify(relatorio, null, 1));
