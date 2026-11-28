// Oráculo do `metaDatasGamma()`, do `extrairMetaEsporteEvento()` (com
// `tokenIdsDoMercado()`) e do `resolverDataInicio()` para o porte no Rust
// (`gamma.rs`). Os eventos da Gamma são remontados a partir do cache de
// metadados do banco (`polymarket_metadata_cache`, exportado em JSONL com
// `{slug, data}`), com as variações de formato que a Gamma usa; as datas de
// início usam as apostas do ledger (`GET /api/history/closed`) com os mapas
// `startBySlug`/`metasBySlug` do mesmo cache.
//
//   node oraculo_gamma.mjs <pasta ui> <despejo .jsonl> <metadados .jsonl> <saída .jsonl>
//
// Depois, no Rust: BETTY_ORACULO_GAMMA=<saída> cargo test --lib -- --ignored oraculo_gamma
// A saída tem dados reais da conta: apague no fim.
import fs from 'node:fs';
import { carregarUi, funcaoDoOraculo, apostasDoDespejo, gravarCasos, espacosDoJs } from './contexto_ui.mjs';

const [ui, despejo, arquivoMetadados, saida] = process.argv.slice(2);
const ctx = carregarUi(ui);
const chamar = funcaoDoOraculo(ctx, '__gamma', '', `e.f === 'meta' ? metaDatasGamma(e.evento)
  : e.f === 'esporte' ? extrairMetaEsporteEvento(e.evento)
  : resolverDataInicio(e.d, e.inicios, e.metas)`);

const metadados = fs.readFileSync(arquivoMetadados, 'utf8').split('\n').filter(Boolean).map(l => JSON.parse(l));
const porSlug = new Map(metadados.map(m => [m.slug, m.data]));
const apostas = await apostasDoDespejo(despejo);
const casos = [];
const juntar = (id, entrada) => casos.push({ id, entrada });

// Um evento da Gamma remontado do cache: datas do evento e de cada mercado,
// textos e mercados do metadado de esporte, com os token IDs em cada um dos
// formatos que a Gamma manda (texto JSON, lista, texto com vírgulas, `tokens`).
function eventoDe(slug, data, i, limite = Infinity) {
  const ev = data.dates?.event || {};
  const sm = data.sport || {};
  const textos = sm.eventTexts || sm.texts || [];
  const mercados = (sm.markets || []).slice(0, limite).map((m, k) => {
    const forma = (i + k) % 6;
    const ids = m.tokenIds || [];
    return {
      ...(data.dates?.markets?.[m.slug] || {}),
      id: String(500000 + k),
      question: m.question, slug: m.slug,
      ...(forma === 3 ? { condition_id: m.conditionId } : { conditionId: m.conditionId }),
      groupItemTitle: m.groupItemTitle, sportsMarketType: m.sportsMarketType,
      description: m.description ?? (k % 3 ? undefined : `Regras de ${m.question}`),
      ...(forma === 1 ? { groupItemThreshold: m.line } : { line: m.line }),
      category: m.category,
      ...(forma === 0 ? { clobTokenIds: JSON.stringify(ids) } : forma === 1 ? { clobTokenIds: ids } : forma === 2 ? { clobTokenIds: ids.join(', ') } : {}),
      ...(forma === 3 ? { tokenIds: ids } : {}),
      ...(forma === 4 ? { tokens: ids.map((t, j) => (j % 2 ? { tokenId: t } : { token_id: t, outcome: 'Yes' })) } : {}),
      ...(forma === 5 ? { clobTokenIds: JSON.stringify(ids), tokens: ids.map(t => ({ id: t })) } : {}),
      ...(k % 2 ? { tags: [{ label: 'Sports', slug: 'sports' }, null, { label: '  ' }], categories: [{ label: m.category, slug: String(m.category || '').toLowerCase() }] } : {}),
    };
  });
  return {
    id: sm.eventId, slug, ticker: slug, title: ev.title, subtitle: i % 4 ? undefined : `${ev.title} ao vivo`, description: `Evento ${ev.title}`,
    category: textos[3], subcategory: textos[5], seriesSlug: sm.seriesSlug,
    startTime: ev.startTime, startDate: ev.startDate, gameStartTime: ev.gameStartTime, eventDate: ev.eventDate, endDate: ev.endDate,
    closedTime: ev.closedTime, umaEndDate: ev.umaEndDate, creationDate: ev.creationDate, acceptingOrdersTimestamp: ev.acceptingOrdersTimestamp,
    tags: textos.slice(2, 9).map((t, j) => (j % 3 ? { label: t, slug: String(t).toLowerCase() } : { label: t })),
    categories: i % 3 ? undefined : [{ label: textos[4], slug: textos[5], parentCategory: 'Sports' }],
    series: i % 2 ? [{ title: textos[textos.length - 1], slug: sm.seriesSlug, ticker: sm.seriesSlug, description: '', tags: [{ label: textos[4] }], categories: [{ slug: 'games' }] }] : undefined,
    markets: mercados,
  };
}

metadados.forEach(({ slug, data }, i) => {
  const evento = eventoDe(slug, data, i);
  juntar(`meta:${i}`, { f: 'meta', evento });
  juntar(`esporte:${i}`, { f: 'esporte', evento });
  const curto = eventoDe(slug, data, i + 1, 12);
  const variante = { ...curto, title: espacosDoJs(curto.title), slug: i % 5 ? curto.slug : 0, id: i % 7 ? curto.id : 0, markets: [...curto.markets, null, 'texto', 7, { slug: curto.markets[0]?.slug, question: 'Repetido', title: '' }] };
  juntar(`meta-variante:${i}`, { f: 'meta', evento: variante });
  juntar(`esporte-variante:${i}`, { f: 'esporte', evento: variante });
});

// Mapas da interface: `startBySlug` e `metasBySlug` (só a entrada que a
// função lê: o evento e o mercado da aposta).
const inicioDe = slug => (porSlug.get(slug)?.start);
const metasDe = (eventSlug, slug) => {
  const dates = porSlug.get(eventSlug)?.dates;
  if (!dates) return {};
  const markets = {};
  if (dates.markets && slug in dates.markets) markets[slug] = dates.markets[slug];
  return { [eventSlug]: { event: dates.event, markets } };
};
const horas = (iso, h) => new Date(Date.parse(iso) + h * 3600000).toISOString();
apostas.forEach((a, i) => {
  const ev = String(a.eventSlug ?? 'undefined');
  const inicios = porSlug.has(ev) ? { [ev]: inicioDe(ev) } : {};
  const metas = metasDe(ev, String(a.slug));
  juntar(`inicio:${i}`, { f: 'inicio', d: a, inicios, metas });
  juntar(`inicio-persistido:${i}`, { f: 'inicio', d: a, inicios: a.eventSlug && a._startDate ? { ...inicios, [a.eventSlug]: a._startDate } : inicios, metas });
  juntar(`inicio-sem-mapas:${i}`, { f: 'inicio', d: a, inicios: {} });
  // Início administrativo: o startDate do mercado é a data de abertura dele.
  const base = Date.parse(a._startDate) > 0 ? new Date(Date.parse(a._startDate)).toISOString() : '2026-06-01T12:00:00Z';
  const market = {
    startDate: horas(base, (i % 5) - 2), creationDate: i % 3 ? horas(base, 1) : null, acceptingOrdersTimestamp: horas(base, -(i % 30)),
    gameStartTime: i % 4 === 0 ? horas(base, 72) : i % 4 === 1 ? '2026-10-20 23:00:00+00' : null,
    eventDate: i % 6 === 0 ? base.slice(0, 10) : null,
    endDate: i % 2 ? horas(base, 24 * (i % 20)) : null, description: i % 3 ? 'Who will be the champion?' : 'Will they reach the final?',
  };
  const event = { title: i % 2 ? `${a.title} winner` : a.title, startTime: horas(base, -1), startDate: base, creationDate: horas(base, -(i % 40)), endDate: horas(base, 24 * 9), description: 'Tournament championship' };
  const metasSuspeitas = { [ev]: { event, markets: { [String(a.slug)]: market } } };
  juntar(`inicio-administrativo:${i}`, { f: 'inicio', d: { ...a, _startDate: base }, inicios: { [ev]: base }, metas: metasSuspeitas });
  juntar(`inicio-administrativo-sem-jogo:${i}`, { f: 'inicio', d: { ...a, _startDate: base, endDate: horas(base, 24 * 30) }, inicios: { [ev]: base },
    metas: { [ev]: { event: { ...event, gameStartTime: null, eventDate: null }, markets: { [String(a.slug)]: { ...market, gameStartTime: null, eventDate: null } } } } });
});

// Casos de borda: sobrescritas, Copa do Mundo, slugs e datas em formatos que o V8 lê.
const datas = ['2026-10-20 23:00:00+00', '2026-10-20 23:00:00', '2026-10-20', '2026-1-5', '2026-10-20T23:00:00.695625Z', '2026-10-20T23:00Z', '2026-10-20t23:00:00z',
  '2026-10-20T23:00:00+0300', '2026-10-20T23:00:00+03', '2026-10-20T23:00:00+00Z', '1789059900', '2026', '2026-10', 'Tue, 20 Oct 2026 23:00:00 GMT',
  '2026-10-20T25:00:00Z', '2026-02-30', '2026-10-20 23:00', '2026-10-20T23:00:00.5+01:00', ' 2026-10-20 ', '2026-10-20\u0085', '2026-10-20T24:00:00Z',
  '2026-02-31', '2026-02-32', '2026-13-01', '2026-04-31T10:00:00Z', '2026-10-20T24:00:01Z', '2026-10-20 23:00:00+03', '2026-10-20 23:00:00+3',
  '2026-10-20 23:00:00+0300', '2026-10-20 23:00:00 GMT', '2026-10-20 23:00:00 UTC', '2026-10-20T23:00:00,5Z', '2026-10-20T23Z', '+002026-10-20',
  '2026-10-20 23:00:00.5', '2026-10-20 23:00:00Z', '2026-1-5 10:00', '2026-01-05T10:00:00+05:30', '2026-10-20T23:00:00-0000', '2026-10-20T23:00',
  '2026-10-20 24:00:00', '2028-02-30', '2026-10-20T23:00:00+24:00', '2026-10-20T23:00:00+23:59', '2026-10-20 23:00:00+2400', '2026-10-20  23:00:00Z',
  '2026-10-20T1:00:00Z', '2026-10-20 1:00:00Z', '2026-10-20 23:00:00 +0000', 1789059900, 0, true, ['2026-10-20'], { data: 1 }, '﻿2026-10-20T10:00:00Z'];
const borda = [];
for (const [k, data] of datas.entries()) {
  const d = { eventSlug: 'ev', slug: 'm', title: 'Who will be the winner?', _startDate: data };
  borda.push({ f: 'inicio', d, inicios: {} });
  borda.push({ f: 'inicio', d: { ...d, _startDate: '2026-10-20T12:00:00Z' }, inicios: { ev: '2026-10-20T12:00:00Z' },
    metas: { ev: { event: { startDate: '2026-10-20T13:00:00Z', creationDate: data, endDate: k % 2 ? '2026-11-20T00:00:00Z' : data }, markets: { m: { gameStartTime: k % 3 ? null : data } } } } });
}
borda.push(
  { f: 'inicio', d: { eventSlug: 'will-neymar-play-in-the-world-cup', slug: 'x' }, inicios: {} },
  { f: 'inicio', d: { eventSlug: 'world-cup-nation-to-reach-quarterfinals', slug: 'will-france-reach-the-quarterfinals-at-the-2026-fifa-world-cup-20260602145134499' }, inicios: {} },
  { f: 'inicio', d: { eventSlug: 'world-cup-winner', slug: 'w', title: 'Will Brazil win the 2026 FIFA World Cup?', endDate: '2026-07-19T19:00:00Z' }, inicios: {} },
  { f: 'inicio', d: { eventSlug: 'world-cup-winner', slug: 'w', title: 'Will Brazil win the 2026 FIFA World Cup', endDate: '2026-07-19T19:00:00Z' }, inicios: {} },
  { f: 'inicio', d: { eventSlug: 5, slug: 'x-2026-01-02' }, inicios: {} },
  { f: 'inicio', d: { eventSlug: 5, slug: 'x' }, inicios: { 5: '2026-01-01T00:00:00Z' } },
  { f: 'inicio', d: { eventSlug: '', slug: ['a-2026-01-02'] }, inicios: {} },
  { f: 'inicio', d: { slug: 'nba-a-b-2026-03-04' }, inicios: {}, metas: { undefined: { event: { startTime: '2026-03-04T15:00:00Z', creationDate: '2026-03-04T16:00:00Z' }, markets: 'texto' } } },
  { f: 'inicio', d: { eventSlug: 'ev', slug: 1 }, inicios: { ev: '2026-03-04T15:00:00Z' }, metas: { ev: { event: 'texto', markets: ['a', { startDate: '2026-03-04T15:00:00Z', creationDate: '2026-03-04T15:30:00Z', eventDate: '2026-03-10' }] } } },
  { f: 'inicio', d: { eventSlug: 'ev', slug: 'm', endDate: '2026-03-20T00:00:00Z', title: 'Who will qualify as champion?' }, inicios: { ev: '2026-03-04T15:00:00Z' }, metas: { ev: { event: { startDate: '2026-03-04T15:00:00Z', creationDate: '2026-03-04T15:00:00Z' }, markets: {} } } },
  { f: 'inicio', d: { eventSlug: 'ev', slug: 'm', endDate: '2026-03-20T00:00:00Z', title: 'Who will be champion?' }, inicios: { ev: 1789059900 }, metas: { ev: { event: { startDate: 1789059900, creationDate: 1789059900 }, markets: {} } } },
  { f: 'inicio', d: null, inicios: {} }, { f: 'inicio', d: 'texto', inicios: {} },
  { f: 'meta' }, { f: 'meta', evento: null }, { f: 'meta', evento: 'texto' }, { f: 'meta', evento: { markets: 5 } }, { f: 'meta', evento: { markets: {} } },
  { f: 'meta', evento: { markets: 'abc', title: 0, question: 'Q' } }, { f: 'meta', evento: { markets: true } },
  { f: 'meta', evento: { markets: [{ slug: '10' }, { slug: '2' }, { slug: 'b' }, { slug: '__proto__', title: 'x' }, { slug: 1.5 }, { slug: ['a', 'b'] }, { slug: '01' }, { slug: '4294967295' }, { slug: '4294967294' }, { slug: { a: 1 } }, { slug: 'b', question: 'Q', description: 0, startTime: 0 }] } },
  { f: 'esporte' }, { f: 'esporte', evento: 0 }, { f: 'esporte', evento: 'texto' }, { f: 'esporte', evento: [1] },
  { f: 'esporte', evento: { id: 0, title: ['a', null, 'b'], category: { x: 1 }, slug: 12345678901234567890, ticker: ' ﻿t\u0085 ', tags: 'não é lista', series: [null, 'x', { tags: [{ label: 1e21 }], categories: 'x' }],
    markets: [null, 'texto', { sportsMarketType: '  totals  ', line: 0, groupItemThreshold: 5, clobTokenIds: '[1, "2", null, [3, 4], {"a":1}, 1e400, 12345678901234567890, true]' },
      { clobTokenIds: '"um"', tokenIds: ' a ,b,,a ', tokens: [null, { token_id: null, tokenId: undefined, id: 0 }, { id: false }] },
      { clobTokenIds: '{"a":1}', tokenIds: '[x', line: null, groupItemThreshold: null }, { clobTokenIds: ' ', tokenIds: '﻿[1]' }, { clobTokenIds: 5, tokenIds: { a: 1 }, tokens: 'x' }] } },
);
borda.forEach((b, i) => juntar(`borda:${i}`, b));

const erros = gravarCasos(saida, casos, chamar);
console.log(`${casos.length} casos (${metadados.length} eventos, ${apostas.length} apostas do ledger, ${borda.length} de borda), ${erros} com exceção no JS`);
