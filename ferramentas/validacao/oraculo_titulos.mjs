// Oráculo do `tituloExibicao()` (com `normalizarNomeMercadoPolymarket()` e
// `normalizarNomeMercadoKalshi()`) para o porte no Rust (`exibicao.rs`).
// Roda a função real sobre as apostas do ledger como a interface as recebe
// (`GET /api/history/closed`), mais variantes que passam pelos ramos de cada
// formato de mercado e casos de borda, e grava a resposta do JS de cada caso.
// O cache `_tituloExibicaoCache` é limpo antes de cada caso (ver `exibicao.rs`).
//
//   node oraculo_titulos.mjs <pasta ui> <despejo .jsonl com GET /api/history/closed> <saída .jsonl>
//
// Depois, no Rust: BETTY_ORACULO_TITULOS=<saída> cargo test --lib -- --ignored oraculo_titulos_exibicao
// A saída tem dados reais da conta: apague no fim.
import { carregarUi, funcaoDoOraculo, apostasDoDespejo, gravarCasos, espacosDoJs } from './contexto_ui.mjs';

const [ui, despejo, saida] = process.argv.slice(2);
const ctx = carregarUi(ui);
const titulo = funcaoDoOraculo(ctx, '__titulo', '_tituloExibicaoCache.clear();', 'tituloExibicao(e)');

const apostas = await apostasDoDespejo(despejo);
const n = apostas.length;
const casos = [];
const juntar = (id, entrada) => casos.push({ id, entrada });
const times = t => {
  const m = String(t || '').match(/^(.+?)\s+vs\.?\s+(.+?)(?:\s*:|\s+-\s+|$)/i);
  return m ? [m[1].trim(), m[2].trim()] : ['Lakers', 'Celtics'];
};

// Perguntas e tipos de mercado da Polymarket que passam pelos ramos de
// `normalizarNomeMercadoPolymarket()` e de `extendedTitle()`.
const modelosPoly = [
  ['spreads', (a, b) => `Spread: ${a} (-1.5)`],
  ['spreads', (a, b) => `1st Half Spread: ${b} (+2.5)`],
  ['spread', (a, b) => `${a} vs. ${b}: Spread ${a} -3.5`],
  ['totals', (a, b) => `${a} vs. ${b}: O/U 2.5`],
  ['team_totals', (a, b) => `Will ${a} score over 1.5 goals?`],
  ['team_totals', (a, b) => `${b} O/U 3.5`],
  ['totals', (a, b) => `${a} vs. ${b}: 1st Half O/U 1.5`],
  ['baseball_team_first_five_total', (a, b) => `${a} vs. ${b}: First 5 Innings O/U 4.5 runs`],
  ['moneyline', (a, b) => `${a} vs. ${b}`],
  ['moneyline', (a, b) => `ATP Masters: ${a} vs. ${b}`],
  ['moneyline', (a, b) => `Will ${a} win the tournament?`],
  ['tennis_set_totals', (a, b) => `${a} vs. ${b}: Total Sets O/U 2.5`],
  ['tennis_match_totals', (a, b) => `${a} vs. ${b}: Match O/U 22.5`],
  ['tennis_first_set_totals', (a, b) => `${a} vs. ${b}: Set 1 Games O/U 9.5`],
  ['tennis_set_handicap', (a, b) => `Set Handicap: ${a} (-1.5) vs. ${b} (+1.5)`],
  ['tennis_first_set_games_totals', (a, b) => `${a} vs. ${b}: Set 1 Games O/U 9.5`],
  ['tennis_set_winner', (a, b) => `Will ${a} win Set 2?`],
  ['rebounds', (a, b) => `${a}: Rebounds O/U 8.5`],
  ['map_handicap', (a, b) => `Map Handicap: ${a} (-1.5) vs. ${b} (+1.5)`],
  ['child_moneyline', (a, b) => `Will ${a} win Map 2?`],
  ['both_teams_to_score', (a, b) => `${a} vs. ${b}: Both Teams to Score`],
  ['both_teams_to_score_first_half', (a, b) => `${a} vs. ${b}: Both Teams to Score in the 1st Half`],
  ['soccer_halftime_result', (a, b) => `Will ${a} be leading at halftime?`],
  ['first_half_moneyline', (a, b) => `Will ${a} win the 1st half?`],
  ['moneyline', (a, b) => `Will ${a} vs. ${b} end in a draw?`],
  ['soccer_halftime_result', (a, b) => `Will ${a} vs. ${b} end in a draw at halftime?`],
  ['baseball_game_extra_innings', (a, b) => `Will the game go to extra innings?: ${a} vs. ${b}`],
  ['nrfi', (a, b) => `${a} vs. ${b}: First Inning Run?`],
  ['totals', (a, b) => `${a} vs. ${b}: Will the 1st inning end in a tie?`],
  ['ufc_method_of_victory', (a, b) => `Will ${a} win by KO/TKO?`],
  ['totals', (a, b) => `UFC 300: ${a} vs. ${b}: O/U 2.5 Rounds`],
  ['f1_race_podium', (a, b) => `Will ${a} finish on the podium?`],
  ['soccer_player_shots_on_target', (a, b) => `${a}: Shots on Target O/U 1.5`],
  ['assists', (a, b) => `${a}: Assists O/U 5.5`],
  ['cricket_first_inning_runs', (a, b) => `${a} vs. ${b}: 1st Innings O/U 160.5`],
  ['basketball_race_to_points', (a, b) => `Will ${a} reach 20 points first?`],
  ['football_first_td', (a, b) => `Will ${a} score the first touchdown?`],
  ['football_team_first_half_winner', (a, b) => `Will ${a} win the 1st half?`],
  ['hockey_team_period1_winner', (a, b) => `Will ${a} win the 1st period?`],
  ['baseball_team_inning3_winner', (a, b) => `Will ${a} win the 3rd inning?`],
  ['soccer_team_to_advance', (a, b) => `${a} vs. ${b}: Will ${a} advance?`],
  ['totals', (a, b) => `${a} vs. ${b}: Resolve to ${a} if ${b} withdraws`],
  ['tipo_desconhecido', (a, b) => `${a} vs. ${b}: Algo novo`],
  ['totals', (a, b) => `${a} vs. ${b}: O/U 9.5 Corners`],
  [' Moneyline ', (a, b) => `${a} vs. ${b}`],
];
const lados = (a, b) => [a, b, 'Yes', 'No', 'Over', 'Under', `${a} -1.5`, `No ${b}`];

// Títulos de mercado da Kalshi para `kalshiDetail()`, `kalshiSpecialTitle()` e o
// título bruto de `normalizarNomeMercadoKalshi()`.
const modelosKalshi = [
  (a, b) => `${a} wins`,
  (a, b) => `Will ${a} win the 1st quarter?`,
  (a, b) => `Will ${a} win the 2nd half?`,
  (a, b) => `First 5 innings: Over 4.5 runs`,
  (a, b) => `Over 2.5 goals scored`,
  (a, b) => `${a}: Over 1.5 runs`,
  (a, b) => `Will ${a} win by KO?`,
  (a, b) => `Will ${a} reach 10 points first?`,
  (a, b) => `${a} vs ${b} Winner?`,
  (a, b) => `Will the game go to extra innings?`,
  (a, b) => `First Inning Run?`,
  (a, b) => `${a} to advance`,
  (a, b) => `Will ${a} win the 3rd inning?`,
  (a, b) => `Will ${a} win Set 2?`,
  (a, b) => `${a}: Shots on target Over 1.5`,
  (a, b) => `Will the teams tie in the 1st period?`,
  (a, b) => `Will ${a} win?`,
];
const titulosKalshi = [
  (a, b) => `${a} vs ${b} - ${a} wins 1st Half`,
  (a, b) => `${a} vs ${b} - ${b} advances`,
  (a, b) => `${a} vs ${b} - Reg Time: ${a}`,
  (a, b) => `${a} vs ${b} - Over 4.5 runs in the first 5 innings`,
  (a, b) => `${a} vs ${b} - ${b} over 3.5 runs scored`,
  (a, b) => `${a} vs ${b} - Over 4.5`,
  (a, b) => `${a} vs ${b} - ${a}`,
  (a, b) => `${a} vs ${b} - Will over 2.5 goals be scored?: Yes`,
  (a, b) => `${a} vs ${b} - Total Points: Yes`,
  (a, b) => `Will over 2.5 goals be scored?`,
  (a, b) => `${a.slice(2)} vs ${b} - Over 2.5 goals`,
];
const tickersKalshi = ['KXNFLGAME-26SEP13KCBUF-KC', 'KXMLBINNINGWIN-26SEP10HOUPHI-3-HOU', 'KXNBATOTAL-26OCT21LALBOS-220', 'KXMLBF5TOTAL-26SEP101305HOUPHI-2'];

apostas.forEach((a, i) => {
  const outra = apostas[(i + 7) % n], terceira = apostas[(i + 13) % n];
  const [ta, tb] = times(a.title);
  juntar(`ledger:${i}`, a);
  const { _sportMeta, _kalshiTitleMeta, ...semMeta } = a;
  juntar(`sem-meta:${i}`, semMeta);
  juntar(`titulo-de-outra:${i}`, { ...a, title: outra.title });
  juntar(`resultado-de-outra:${i}`, { ...a, outcome: outra.outcome });
  juntar(`resultado-variado:${i}`, { ...a, outcome: lados(ta, tb)[i % 8] });
  juntar(`outra-casa:${i}`, { ...a, _platform: a._platform === 'kalshi' ? 'polymarket' : 'kalshi' });
  juntar(`exibido-de-outra:${i}`, { ...a, _titleDisplay: outra.title });
  juntar(`ids-de-outra:${i}`, { ...a, slug: terceira.slug, conditionId: terceira.conditionId, asset: terceira.asset, eventSlug: terceira.eventSlug });
  juntar(`espacos:${i}`, { ...a, title: espacosDoJs(a.title), outcome: espacosDoJs(a.outcome) });
  if (_sportMeta && typeof _sportMeta === 'object') {
    juntar(`meta-de-outra:${i}`, { ...a, _sportMeta: outra._sportMeta ?? terceira._sportMeta });
    juntar(`sem-asset:${i}`, { ...a, asset: undefined });
    // Todos os mercados do evento com uma pergunta e um tipo de modelo: o
    // exato (por asset, conditionId ou slug) passa a ser esse.
    const [tipo, pergunta] = modelosPoly[i % modelosPoly.length];
    const mercados = (_sportMeta.markets || []).map(m => ({ ...m, question: pergunta(ta, tb), sportsMarketType: tipo, groupItemTitle: i % 3 ? m.groupItemTitle : `${ta} -1.5` }));
    for (const [k, lado] of lados(ta, tb).entries()) {
      if ((i + k) % 3) continue;
      juntar(`modelo-poly:${i}:${k}`, { ...a, outcome: lado, _sportMeta: { ..._sportMeta, markets: mercados } });
    }
    juntar(`modelo-poly-espacos:${i}`, { ...a, _sportMeta: { ..._sportMeta, markets: mercados.map(m => ({ ...m, question: espacosDoJs(m.question) })) } });
  } else if (a._platform !== 'kalshi') {
    juntar(`poly-legado:${i}`, { ...a, _platform: 'polymarket', slug: ['atp-ab-cd-2026-01-12', 'nfl-kc-buf-2026-09-13', 'mlb-nyy-bos-2026-04-01'][i % 3], title: `${ta} vs. ${tb}: O/U 9.5 Corners`, outcome: ['Over', ta, 'Under'][i % 3] });
  }
  if (_kalshiTitleMeta && typeof _kalshiTitleMeta === 'object') {
    juntar(`kmeta-de-outra:${i}`, { ...a, _kalshiTitleMeta: outra._kalshiTitleMeta ?? terceira._kalshiTitleMeta });
    juntar(`versao-formato:${i}`, { ...a, _kalshiTitleFormatVersion: Number(a._kalshiTitleFormatVersion || 0) >= 4 ? 0 : 4 });
    const { scope, ...semEscopo } = _kalshiTitleMeta;
    juntar(`sem-escopo:${i}`, { ...a, _kalshiTitleMeta: semEscopo });
    const market = { ..._kalshiTitleMeta.market, title: modelosKalshi[i % modelosKalshi.length](ta, tb), yes_sub_title: i % 2 ? ta : _kalshiTitleMeta.market?.yes_sub_title };
    const ticker = tickersKalshi[i % tickersKalshi.length];
    juntar(`modelo-kalshi:${i}`, { ...a, outcome: ['yes', 'no', 'Yes', 'No'][i % 4], _kalshiTitleMeta: { ..._kalshiTitleMeta, market } });
    juntar(`modelo-kalshi-ticker:${i}`, { ...a, slug: ticker, ticker, outcome: ['no', 'yes'][i % 2], _kalshiTitleMeta: { ..._kalshiTitleMeta, market } });
    juntar(`titulo-kalshi:${i}`, { ...semMeta, title: titulosKalshi[i % titulosKalshi.length](ta, tb), outcome: ['Yes', 'No', ''][i % 3], _kalshiTitleFormatVersion: i % 5 ? 3 : 4 });
  }
});

// Casos de borda: exceções, tipos estranhos, espaços e unidades UTF-16.
const mercado = (extra = {}) => ({ slug: 'm', question: 'Lakers vs. Celtics: O/U 220.5', sportsMarketType: 'totals', ...extra });
const borda = [
  null, 'texto', 5, [], [1, 2], true, {}, { title: 5 }, { title: ['Lakers vs Celtics', 'x'] }, { _titleDisplay: 7 },
  { _platform: 'predict', _predictTitle: 'Predito' }, { _platform: 'predict', _predictTitle: 0, title: 'T' }, { _platform: ['kalshi'], title: 'A vs B - A' },
  { _platform: 'kalshi', _titleDisplay: 'Over 2.5 goals', sport: 'Soccer', title: 'A vs B - Over 2.5 goals' },
  { _platform: 'kalshi', _titleDisplay: 'Lakers wins?', sport: 'Basketball' },
  { slug: 'm', _sportMeta: { markets: [null] } },
  { slug: 'm', asset: '1', _sportMeta: { markets: [{ tokenIds: 'abc' }] } },
  { slug: 'm', asset: '1', _sportMeta: { markets: [{ tokenIds: 5 }] } },
  { slug: 'm', asset: '"1"', outcome: 'Over', title: 'A vs B', _sportMeta: { markets: [{ slug: 'm', tokenIds: ['"1"'], question: 'O/U 2.5 Rounds' }] } },
  { slug: 'm', outcome: 'Over', title: 'A vs B', _sportMeta: { texts: 5, markets: [mercado({ question: 'Over 2.5 rounds' })] } },
  { slug: 'm', outcome: 'Yes', title: 'X', _sportMeta: { texts: 'A vs B', markets: [mercado({ question: 'Over 2.5 rounds' })] } },
  { slug: 'm', outcome: 'Yes', title: 'UFC 300: A vs. B - Over 2.5', _sportMeta: { markets: [mercado({ question: 'O/U 2.5 Rounds', sportsMarketType: '' })] } },
  { _platform: 'kalshi', slug: 'KXMLBINNINGWIN-26SEP10HOUPHI-3-HOU', outcome: 'yes', title: 'Houston vs Philadelphia - 3rd Inning Houston ML', _kalshiTitleMeta: { market: { title: 5 } } },
  { _platform: 'kalshi', slug: 'KXMLBINNINGWIN-26SEP10HOUPHI-3-HOU', outcome: 'yes', title: 'Houston vs Philadelphia - 3rd Inning Houston ML', _kalshiTitleMeta: { market: { title: null, yes_sub_title: 'Houston' } } },
  { _platform: 'kalshi', slug: 'KXMLBINNINGWIN-26SEP10HOUPHI-3-HOU', outcome: 'no', title: 'Houston vs Philadelphia - 3rd Inning', _kalshiTitleMeta: { market: 'texto' } },
  { _platform: 'kalshi', title: 'A vs B - A', outcome: 'yes', _kalshiTitleMeta: { market: { title: 'Will A win the 1st half?' }, scope: null, matchup: 'A vs B' } },
  { _platform: 'kalshi', title: 'A vs B - A', outcome: 'yes', _kalshiTitleMeta: { market: { title: 'Will A win the 1st half?' }, scope: 5, matchup: 'A vs B' } },
  { _platform: 'kalshi', title: 'A vs B - A', outcome: 'yes', _kalshiTitleMeta: { market: { title: 'Will A reach 10 points first_q2' }, scope: null } },
  { _platform: 'kalshi', title: 'A vs B - A', outcome: 'yes', _kalshiTitleMeta: { market: { title: 'Will A reach 10 points first_q2' }, scope: '' } },
  { _platform: 'kalshi', title: 'A vs B - A', outcome: 'yes', _kalshiTitleMeta: { market: { title: 'A vs B Winner?', yes_sub_title: 'A' }, scope: null } },
  { _platform: 'kalshi', title: 'A vs B - A', outcome: 'yes', _kalshiTitleMeta: { market: { title: 'A vs B Winner?', yes_sub_title: 'A' }, scope: 'soccer match' } },
  { _platform: 'kalshi', title: 'A vs B', ticker: 'KXNFLGAME-X', outcome: 'no', _kalshiTitleMeta: { market: { title: 'Will A win?' }, matchup: 'Team A vs Team B' } },
  { _platform: 'kalshi', title: 'Ams vs Bos - Ams', outcome: 'no', _kalshiTitleMeta: { market: { title: 'Ams wins?' }, matchup: 'Teams vs Boston' } },
  { _platform: 'kalshi', title: 'Ams vs Bos - Ams', outcome: 'no', _kalshiTitleMeta: 'texto' },
  { _platform: 'kalshi', title: 'İstanbul vs Roma - İstanbul', outcome: 'Yes' },
  { _platform: 'kalshi', title: 'A vs B - Total: No', outcome: 'no', _kalshiTitleFormatVersion: 'x' },
  { _platform: 'polymarket', title: 'Notre Dame vs. Navy', outcome: 'Notre Dame', slug: 'cfb-nd-navy-2026-09-12' },
  { _platform: 'polymarket', title: 'Notre Dame vs. Navy: Spread', outcome: 'No Navy +3.5' },
  { _platform: 'polymarket', title: 'Lakers vs. Celtics - More Markets: Lakers Total Rebounds??', outcome: 'Yes' },
  { _platform: 'polymarket', title: 'Lakers vs. Celtics: Lakers to advance?', outcome: 'Yes' },
  { _platform: 'polymarket', title: 'Lakers vs. Celtics: Who wins?', outcome: 'Lakers wins' },
  { _platform: 'polymarket', title: 'Will Lakers win vs Celtics?', outcome: 'Yes' },
  { _platform: 'polymarket', title: 'Lakers vs Celtics Winner', outcome: 'Celtics' },
  { _platform: 'polymarket', title: 'İİ Lakers vs Celtics extra time', outcome: 'Lakers' },
  { _platform: 'polymarket', title: 'Ünïcödé😀 vs B 😀 — final', outcome: 'B' },
  { _platform: 'polymarket', title: 'A\u0085vs B: Total 2.5', outcome: 'Over' },
  { _platform: 'polymarket', title: 'A﻿vs﻿B: Total 2.5', outcome: 'Over' },
  { _platform: 'polymarket', title: 'A vs B\r: O/U 2.5', outcome: 'Under', slug: 'mlb-a-b-2026-01-01' },
  { _platform: 'polymarket', title: 'A vs B: Over 2.5', outcome: 'Over', sport: ['baseball'], eventSlug: { x: 1 } },
  { _platform: 'polymarket', title: 'A vs B', outcome: 'A', slug: 'm', _sportMeta: { texts: ['Liga: A vs B\n- Will x', 7, null], markets: [mercado({ question: null, sportsMarketType: 'moneyline', line: '1,5' })] } },
  { _platform: 'polymarket', title: 'A vs B', outcome: 'A', slug: 'm', _sportMeta: { markets: [mercado({ sportsMarketType: 'spreads', question: 'Spread', groupItemTitle: 'A +1,5', line: 1.5 })] } },
  { _platform: 'polymarket', title: 'A vs B', outcome: 'B', slug: 'm', _sportMeta: { markets: [mercado({ sportsMarketType: 'spreads', question: 'Spread', groupItemTitle: 'A', line: '-0' })] } },
  { _platform: 'polymarket', title: 'A vs B', outcome: 'Over', slug: 'm', _sportMeta: { markets: [mercado({ question: 'Will A score over 1.5 runs?', category: 7 })] } },
  { _platform: 'polymarket', title: 'Sporting vs Benfica', outcome: 'Sporting', slug: 'm', _sportMeta: { markets: [mercado({ sportsMarketType: 'moneyline', question: 'Sporting vs Benfica', category: 'Tennis' })] } },
  { _platform: 'polymarket', title: 'A vs B', outcome: 'Yes', slug: 'm', _sportMeta: { markets: [mercado({ sportsMarketType: 'moneyline', question: 'Will A vs. B end in a draw in the second half?' })] } },
  { _platform: 'polymarket', title: 'A vs B', outcome: 'Yes', slug: 'm', _sportMeta: { markets: [mercado({ sportsMarketType: 'moneyline', question: 'Will A vs. B end in a draw at the banana?' })] } },
  { _platform: 'polymarket', title: '  vs B: Spread', outcome: 'B +1.5' },
  { _platform: 'polymarket', title: 'A vs B', outcome: 'T', slug: 'm', _sportMeta: { markets: [mercado({ sportsMarketType: 'map_handicap', question: 'Map Handicap: ＴＥＡＭ​ (-1.5) vs. B (+1.5)' })] } },
  { _platform: 'polymarket', title: 'A vs B', outcome: 'TEAM', slug: 'm', _sportMeta: { markets: [mercado({ sportsMarketType: 'map_handicap', question: 'Map Handicap: ＴＥＡＭ​ (-1.5) vs. B (+1.5)' })] } },
  { _platform: 'polymarket', title: 'A vs B', outcome: 'X', slug: 'm', _sportMeta: { markets: [mercado({ slug: 'M ', question: 'x' }), mercado({ slug: 'm', question: 'y' })] } },
  { _platform: 'sxbet', title: 'SX title' }, { _platform: 'outra', title: '' },
];
borda.forEach((b, i) => juntar(`borda:${i}`, b));

const erros = gravarCasos(saida, casos, titulo);
console.log(`${casos.length} casos (${n} apostas do ledger, ${borda.length} de borda), ${erros} com exceção no JS`);
