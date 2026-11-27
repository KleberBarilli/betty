// Mede o Betty aberto pelo CDP do WebView2 (porta da depuração remota), sem
// capturas de tela: só tempos e contagens, nenhum dado da conta.
//
//   node medir_app.mjs <porta> abertura   linha do tempo desde que a página existe:
//                                          tela pronta, dados salvos, confirmação
//   node medir_app.mjs <porta> scripts    recarrega a página com rastreamento e soma,
//                                          por arquivo, compilação e execução dos scripts
//   node medir_app.mjs <porta> marcar     grava um marcador na página (window.__marca)
//   node medir_app.mjs <porta> conferir   diz se o marcador ainda está lá (a página
//                                          não recarregou)
//
// A porta vem de WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=<porta>.
const [porta, modo] = process.argv.slice(2);
const inicio = Date.now();
const ms = () => Date.now() - inicio;
const dormir = t => new Promise(r => setTimeout(r, t));

let alvo = null;
while (!alvo) {
  if (ms() > 120_000) throw new Error('WebView sem depuração remota');
  try {
    const lista = await (await fetch(`http://127.0.0.1:${porta}/json/list`)).json();
    alvo = lista.find(t => t.type === 'page' && t.url.includes('betty.localhost'));
  } catch (_) {}
  if (!alvo) await dormir(100);
}
const ws = new WebSocket(alvo.webSocketDebuggerUrl);
await new Promise((ok, erro) => { ws.onopen = ok; ws.onerror = erro; });
let id = 0;
const pendentes = new Map();
const ouvintes = [];
ws.onmessage = ev => {
  const m = JSON.parse(ev.data);
  if (m.id && pendentes.has(m.id)) { pendentes.get(m.id)(m); pendentes.delete(m.id); }
  if (m.method) ouvintes.forEach(f => f(m));
};
const cdp = (method, params = {}) => new Promise(r => { const n = ++id; pendentes.set(n, r); ws.send(JSON.stringify({ id: n, method, params })); });
const avaliar = async expr => (await cdp('Runtime.evaluate', { expression: expr, returnByValue: true })).result?.result?.value;

const ESTADO = `JSON.stringify({
  carregando: document.body?.classList.contains('initial-loading') ?? true,
  salvos: typeof exibindoDadosSalvos === 'undefined' ? null : exibindoDadosSalvos,
  selo: document.getElementById('seloDadosSalvos') ? !document.getElementById('seloDadosSalvos').hidden : null,
  aviso: document.getElementById('seloDadosSalvos')?.classList.contains('aviso') || false,
  desdeNavegacao: Math.round(performance.now())
})`;

async function abertura() {
  const marcos = {};
  let antes = null;
  while (ms() < 240_000) {
    let e;
    try { e = JSON.parse(await avaliar(ESTADO)); } catch (_) { await dormir(100); continue; }
    const t = e.desdeNavegacao;
    if (!marcos.primeiraLeitura) marcos.primeiraLeitura = t;
    if (!e.carregando && !marcos.telaPronta) marcos.telaPronta = t;
    if (e.salvos && e.selo && !marcos.dadosSalvos) marcos.dadosSalvos = t;
    if (antes?.salvos && e.salvos === false && !e.selo) { marcos.confirmados = t; break; }
    if (e.aviso) { marcos.aviso = t; break; }
    // Primeira abertura, sem dados salvos: a tela só aparece com os confirmados.
    if (!e.carregando && e.salvos === false && !e.selo && !marcos.dadosSalvos && marcos.telaPronta && t - marcos.telaPronta > 3000) {
      marcos.confirmados = marcos.telaPronta;
      marcos.semDadosSalvos = true;
      break;
    }
    antes = e;
    await dormir(100);
  }
  return marcos;
}

async function scripts() {
  const eventos = [];
  ouvintes.push(m => { if (m.method === 'Tracing.dataCollected') eventos.push(...m.params.value); });
  const fim = new Promise(r => ouvintes.push(m => { if (m.method === 'Tracing.tracingComplete') r(); }));
  await cdp('Page.enable');
  const carregou = new Promise(r => ouvintes.push(m => { if (m.method === 'Page.loadEventFired') r(); }));
  await cdp('Tracing.start', {
    transferMode: 'ReportEvents',
    traceConfig: { includedCategories: ['devtools.timeline', 'v8', 'disabled-by-default-v8.compile', 'blink.user_timing'] },
  });
  await cdp('Page.reload', { ignoreCache: false });
  await carregou;
  await dormir(1500);
  await cdp('Tracing.end');
  await fim;
  const porArquivo = {};
  const soma = (url, tipo, dur) => {
    if (!url || !dur) return;
    const nome = url.split('/').pop().split('?')[0] || url;
    porArquivo[nome] ??= { compilar: 0, executar: 0 };
    porArquivo[nome][tipo] += dur / 1000;
  };
  for (const e of eventos) {
    const d = e.args?.data || e.args?.beginData || {};
    if (e.ph !== 'X') continue;
    if (e.name === 'v8.compile' || e.name === 'V8.CompileCode' || e.name === 'v8.compileModule') soma(d.url || e.args?.fileName, 'compilar', e.dur);
    if (e.name === 'EvaluateScript' || e.name === 'v8.evaluateModule') soma(d.url, 'executar', e.dur);
  }
  const arred = o => Object.fromEntries(Object.entries(o).map(([k, v]) => [k, { compilar: +v.compilar.toFixed(1), executar: +v.executar.toFixed(1) }]));
  return { eventos: eventos.length, msPorArquivo: arred(porArquivo) };
}

let resultado;
if (modo === 'abertura') resultado = await abertura();
else if (modo === 'scripts') resultado = await scripts();
else if (modo === 'marcar') resultado = await avaliar(`(window.__marca = ${Date.now()}, 'marcado')`);
else if (modo === 'conferir') resultado = await avaliar(`typeof window.__marca === 'number' ? 'mesma página' : 'página recarregou'`);
else throw new Error(`modo desconhecido: ${modo}`);
console.log(JSON.stringify(resultado));
ws.close();
