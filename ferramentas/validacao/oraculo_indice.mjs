// Oráculo dos auxiliares de `indiceHistoricoFechado()` (modo `ajudantes`:
// `dataInicioAposta`, `getTsResultadoPnL`, `getStake`, `getOddNum`,
// `assinaturaFonteLedger`, `versaoTituloConfirmado`) e da própria função (modo
// `indice`) para o porte no Rust (`indice.rs`). Usa as apostas do ledger como
// a interface as recebe (`GET /api/history/closed`), o mapa `metasDatasPnl`
// montado do cache de metadados (`exportar_metadados.py`), variantes de cada
// campo que as funções leem e casos de borda.
//
//   node oraculo_indice.mjs <ajudantes|indice> <pasta ui> <despejo .jsonl> <metadados .jsonl> <saída .jsonl>
//
// Depois, no Rust: BETTY_ORACULO_AJUDANTES=<saída> cargo test --lib -- --ignored oraculo_ajudantes_indice
//              ou: BETTY_ORACULO_INDICE=<saída> cargo test --lib -- --ignored oraculo_indice
// Números vão como {"n": ...} ("NaN", "Infinity", "-0" em texto) e datas como {"data": ms}.
// A saída tem dados reais da conta: apague no fim.
import fs from 'node:fs';
import vm from 'node:vm';
import { carregarUi, apostasDoDespejo, gravarCasos, espacosDoJs } from './contexto_ui.mjs';

const [modo, ui, despejo, arquivoMetadados, saida] = process.argv.slice(2);
if (!['ajudantes', 'indice'].includes(modo)) throw new Error('modo: ajudantes ou indice');
const ctx = carregarUi(ui);
vm.runInContext(`
  globalThis.__codificar = v => typeof v === 'number'
    ? { n: Number.isNaN(v) ? 'NaN' : !Number.isFinite(v) ? (v > 0 ? 'Infinity' : '-Infinity') : Object.is(v, -0) ? '-0' : v }
    : v instanceof Date ? { data: v.getTime() } : { v: v === undefined ? null : v };
  globalThis.__tentar = f => { try { return __codificar(f()); } catch (x) { return { erro: true }; } };
  globalThis.__caso = texto => {
    const { d, metas } = JSON.parse(texto);
    metasDatasPnl = metas === undefined ? null : metas;
    _tituloExibicaoCache.clear();
    if (${JSON.stringify(modo)} === 'indice') {
      try { return JSON.stringify({ saida: indiceHistoricoFechado(d) }); }
      catch (x) { return JSON.stringify({ erro: String(x?.message || x) }); }
    }
    return JSON.stringify({ saida: {
      dataInicio: __tentar(() => dataInicioAposta(d)),
      tsPnl: __tentar(() => getTsResultadoPnL(d)),
      stake: __tentar(() => getStake(d)),
      odd: __tentar(() => getOddNum(d)),
      assinatura: __tentar(() => assinaturaFonteLedger(d)),
      versao: __tentar(() => versaoTituloConfirmado(d, tituloExibicao(d))),
    } });
  };`, ctx);
const chamar = entrada => JSON.parse(vm.runInContext(`__caso(${JSON.stringify(JSON.stringify(entrada))})`, ctx));

const metadados = new Map(fs.readFileSync(arquivoMetadados, 'utf8').split('\n').filter(Boolean).map(l => JSON.parse(l)).map(m => [m.slug, m.data]));
const apostas = await apostasDoDespejo(despejo);
const n = apostas.length;
const casos = [];
const juntar = (id, d, metas) => casos.push({ id, entrada: metas === undefined ? { d } : { d, metas } });
// `metasDatasPnl` como a sincronização deixa: o mapa `metasBySlug` dos eventos.
const metasDe = a => {
  const m = {};
  for (const slug of [a?.eventSlug, a?.slug]) {
    const dates = metadados.get(String(slug));
    if (dates?.dates) m[slug] = dates.dates;
  }
  return m;
};
const sem = (a, ...chaves) => Object.fromEntries(Object.entries(a).filter(([k]) => !chaves.includes(k)));

const stakes = [
  { initialValue: '5' }, { initialValue: 0, totalBought: 10, avgPrice: '0.25' }, { initialValue: -1, totalBought: '3', avgPrice: 0, size: 4 },
  { initialValue: null, size: '2', avgPrice: 'abc' }, { initialValue: [7] }, { initialValue: true }, { initialValue: {} , totalBought: 2, avgPrice: [0.5] },
  { initialValue: ' 3 ' }, { initialValue: '﻿3' }, { initialValue: '3\u0085' }, { totalBought: 1e308, avgPrice: 10 }, { avgPrice: 5e-324 },
  { avgPrice: '' }, { avgPrice: -0.5 }, { avgPrice: '0x10' }, { avgPrice: [[0.5]] }, { avgPrice: 'Infinity' }, { initialValue: '1e400' },
];
const datas = ['2026-10-20T23:00:00Z', '2026-10-20 23:00:00+00', '2026-10-20', ' 2026-10-20', '2026-10-20 10:00:00', '2026-02-31T10:00:00Z',
  '2026-10-20T23:00:00+03', '1789059900', 1789059900, 1789059900123, '1789059900123', 0, '', '  ', 'x', 946684799, 946684800, 4102444800, 4102444799,
  '1999-12-31T23:59:59Z', '2100-12-31T23:59:59.999Z', '2101-01-01T00:00:00Z', true, [1789059900], { t: 1 }, -1789059900, '2026-10-20T23:00:00.123456789Z', '1e12', 8.64e15, 8.64e15 + 1];
const plataformas = ['KALSHI', 'Polymarket', ['kalshi'], 5, '', null, 'predict', 'sxbet', 'outra', undefined];
const tickers = ['KXMLBTEAMTOTAL-26AUG081805TORPHI', 'KXNFLGAME-26SEP131300KCBUF', 'KXNBAGAME-26feb301930LALBOS', 'KXNHLGAME-26DEC012461BOSMTL',
  'KXEPLGAME-26AUG081805ARSCHE', 'kxmlbgame-26jan052359nyybos', 'KXMLBGAME-26AUG0818', 5];

apostas.forEach((a, i) => {
  const outra = apostas[(i + 7) % n];
  juntar(`ledger:${i}`, a, metasDe(a));
  juntar(`sem-metas:${i}`, a);
  juntar(`sem-resolucao:${i}`, sem(a, '_resolvedTime', '_confirmedClosedTs', '_settledTime'), metasDe(a));
  juntar(`sem-datas:${i}`, sem(a, '_startDate', '_confirmedClosedTs', '_settledTime', '_saleTs', '_lastTradeTs', '_resolvedTime'), metasDe(a));
  juntar(`stake:${i}`, { ...sem(a, 'initialValue', 'totalBought', 'avgPrice', 'size'), ...stakes[i % stakes.length] });
  const data = datas[i % datas.length];
  juntar(`datas:${i}`, { ...a, _startDate: data, endDate: datas[(i + 3) % datas.length], _confirmedClosedTs: i % 4 ? undefined : data, _settledTime: datas[(i + 5) % datas.length],
    _saleTs: i % 3 ? undefined : datas[(i + 7) % datas.length], timestamp: datas[(i + 11) % datas.length], _resolvedTime: i % 5 ? undefined : data });
  juntar(`plataforma:${i}`, { ...a, _platform: plataformas[i % plataformas.length] }, metasDe(a));
  juntar(`perda-resolvida:${i}`, { ...sem(a, '_confirmedClosedTs'), redeemable: true, curPrice: ['0', 0, '', null, ' 0 '][i % 5] }, metasDe(a));
  if (a._platform === 'kalshi') {
    juntar(`kalshi-ticker:${i}`, { ...sem(a, '_startDate'), ticker: tickers[i % tickers.length], slug: i % 2 ? a.slug : tickers[(i + 1) % tickers.length] });
    juntar(`kalshi-versao:${i}`, { ...a, _titleDisplayVersion: i % 3 ? '28' : 28, _titleDisplay: i % 2 ? outra.title : undefined });
    juntar(`kalshi-titulo:${i}`, { ...sem(a, '_kalshiTitleMeta'), title: outra.title, _kalshiTitleMeta: i % 2 ? undefined : { market: { title: 'Lakers wins?' }, scope: i % 3 ? null : 5 } });
  }
  // A assinatura: chaves em outra ordem, outra posição (`_order`) e textos com espaços e controles.
  const embaralhada = Object.fromEntries(Object.entries({ ...a, _order: i * 3 }).reverse());
  juntar(`assinatura:${i}`, i % 2 ? embaralhada : { ...embaralhada, title: espacosDoJs(a.title) + '\u0001\u001f"\\/ \u{1F600}', 'ç': -0, '\u{1F600}': [1e21, 1.50, null], '￿': true });
});

const borda = [
  null, 'texto', 5, [], [1, 'a'], true, {}, '\u{1F600}a', { _order: 1 }, { _platform: 'kalshi' }, { _platform: 'polymarket', redeemable: true, curPrice: 0 },
  { _platform: 'kalshi', _settledTime: '2026-10-20T23:00:00.695625Z', title: 'A vs B - A', _kalshiTitleMeta: 'texto' },
  { _platform: 'kalshi', slug: 'KXMLBINNINGWIN-26SEP10HOUPHI-3-HOU', outcome: 'yes', title: 'Houston vs Philadelphia - 3rd Inning', _kalshiTitleMeta: { market: { title: 5 } } },
  { _platform: 'kalshi', title: { a: 1 }, _titleDisplay: { a: 1 } }, { _platform: 'kalshi', title: { a: 1 } }, { _platform: 'kalshi', title: 7, _titleDisplay: 7 },
  { _platform: 'kalshi', title: 'A vs B - A', _titleDisplay: 'A vs B - A ML', _kalshiTitleMeta: { matchup: 'A vs B' } },
  { _platform: 'predict', _predictTitle: 'P', timestamp: 1789059900, _realizedAt: '2026-10-20T23:00:00Z' },
  { _platform: 'sxbet', _realizedAt: 'x' }, { eventSlug: 'e', slug: 's', redeemable: true, curPrice: 0 },
];
borda.forEach((b, i) => juntar(`borda:${i}`, b, i % 2 ? { e: { markets: { s: { closedTime: '2026-10-21T00:00:00Z' } } }, s: { markets: 'texto' } } : undefined));

const erros = gravarCasos(saida, casos, chamar);
console.log(`${casos.length} casos (${n} apostas do ledger, ${borda.length} de borda), ${erros} com exceção no JS`);
