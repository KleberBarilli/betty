// Acompanha a abertura do Betty pelo CDP do WebView2: linha do tempo do selo e
// das telas, com capturas nos momentos principais.
// uso: node observar_abertura.mjs <porta> <pasta de saída>
import fs from 'node:fs';
import path from 'node:path';

const [porta, saida] = process.argv.slice(2);
const inicio = Date.now();
const ms = () => Date.now() - inicio;
const dormir = t => new Promise(r => setTimeout(r, t));

let alvo = null;
while (!alvo) {
  if (ms() > 90_000) throw new Error('WebView sem depuração remota');
  try {
    const lista = await (await fetch(`http://127.0.0.1:${porta}/json/list`)).json();
    alvo = lista.find(t => t.type === 'page' && t.url.includes('betty.localhost'));
  } catch (_) {}
  if (!alvo) await dormir(150);
}
const ws = new WebSocket(alvo.webSocketDebuggerUrl);
await new Promise((ok, erro) => { ws.onopen = ok; ws.onerror = erro; });
let id = 0;
const pendentes = new Map();
ws.onmessage = ev => {
  const m = JSON.parse(ev.data);
  if (m.id && pendentes.has(m.id)) { pendentes.get(m.id)(m); pendentes.delete(m.id); }
};
const cdp = (method, params = {}) => new Promise(r => { const n = ++id; pendentes.set(n, r); ws.send(JSON.stringify({ id: n, method, params })); });
const avaliar = async expr => (await cdp('Runtime.evaluate', { expression: expr, returnByValue: true })).result?.result?.value;
const capturar = async nome => {
  const r = await cdp('Page.captureScreenshot', { format: 'png' });
  if (r.result?.data) fs.writeFileSync(path.join(saida, nome), Buffer.from(r.result.data, 'base64'));
};
const ESTADO = `(() => {
  const el = id => document.getElementById(id);
  const selo = el('seloDadosSalvos');
  return JSON.stringify({
    carregando: document.body.classList.contains('initial-loading'),
    seloVisivel: selo ? !selo.hidden : null,
    selo: el('seloDadosSalvosTexto')?.textContent || '',
    aviso: selo?.classList.contains('aviso') || false,
    salvos: typeof exibindoDadosSalvos === 'undefined' ? null : exibindoDadosSalvos,
    patrimonio: el('heroPatrimonio')?.textContent || '',
    kpis: (el('stats')?.innerText || '').replace(/\\s+/g, ' ').slice(0, 160),
    grafico: Boolean(el('chartArea')?.querySelector('svg')),
    status: el('statusAtualizado')?.textContent || '',
    progresso: el('syncProgress')?.hidden === false ? el('syncProgressTitle')?.textContent : '',
    secao: typeof secaoAtual === 'undefined' ? '' : secaoAtual
  });
})()`;

const linha = [];
let ultimo = '', fotoSalvo = false, fotoEstat = false, fim = false, inicioSalvos = 0;
while (ms() < 240_000 && !fim) {
  let e;
  try { e = JSON.parse(await avaliar(ESTADO)); } catch (_) { await dormir(200); continue; }
  const chave = JSON.stringify({ ...e, status: undefined });
  if (chave !== ultimo) { linha.push({ ms: ms(), ...e }); ultimo = chave; }
  if (e.salvos && !inicioSalvos) inicioSalvos = ms();
  const pintado = (e.kpis && !/Sincronizando|Carregando/.test(e.kpis)) || e.grafico || (inicioSalvos && ms() - inicioSalvos > 2500);
  if (!fotoSalvo && e.salvos && e.seloVisivel && !e.carregando && pintado) {
    await dormir(400);
    await capturar('1-visao-dados-salvos.png');
    fotoSalvo = true;
    linha.push({ ms: ms(), nota: 'captura 1 (Visão geral com dados salvos)' });
    // Durante a atualização, as Estatísticas também saem do estado salvo.
    await avaliar(`irPara('estatisticas')`);
    await dormir(900);
    const estat = await avaliar(`(document.getElementById('statsEstat')?.innerText || '').replace(/\\s+/g,' ').slice(0,200)`);
    linha.push({ ms: ms(), nota: 'Estatísticas durante a atualização', estat, salvos: await avaliar('exibindoDadosSalvos') });
    await capturar('2-estatisticas-dados-salvos.png');
    fotoEstat = true;
    await avaliar(`irPara('visao')`);
  }
  if (fotoSalvo && e.salvos === false && !e.seloVisivel) {
    await dormir(1500);
    await capturar('3-visao-confirmada.png');
    linha.push({ ms: ms(), nota: 'captura 3 (dados confirmados)', ...JSON.parse(await avaliar(ESTADO)) });
    fim = true;
  }
  if (e.aviso) {
    await dormir(800);
    await capturar('3-visao-aviso.png');
    linha.push({ ms: ms(), nota: 'atualização falhou: selo de aviso' });
    fim = true;
  }
  await dormir(150);
}
fs.writeFileSync(path.join(saida, 'linha-do-tempo.json'), JSON.stringify(linha, null, 1));
console.log(JSON.stringify(linha, null, 1));
ws.close();
