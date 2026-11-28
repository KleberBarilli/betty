// Contexto comum dos oráculos da interface: carrega os scripts de `ui/` num
// `node:vm` com DOM falso (Proxy) e `localStorage` em memória, lê as apostas
// de um despejo de `GET /api/history/closed` e grava os casos com a resposta
// do JS. Cada caso vai ao vm como texto JSON e volta como texto: objetos entre
// realms não se comparam.
import vm from 'node:vm';
import fs from 'node:fs';
import path from 'node:path';
import readline from 'node:readline';

const SCRIPTS = ['audit-utils.js', 'market-title-utils.js', 'market-breakdown.js', 'odds-ranges.js', 'profit-calendar.js', 'acoes.js', 'dashboard.js'];

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

export function carregarUi(ui) {
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
  return ctx;
}

/// Declara no vm `globalThis[nome] = texto => JSON {saida} | {erro}` chamando `corpo` (uma expressão com `e`, a entrada).
export function funcaoDoOraculo(ctx, nome, preparo, corpo) {
  vm.runInContext(`globalThis.${nome} = texto => {
    ${preparo}
    try { const e = JSON.parse(texto); return JSON.stringify({ saida: ${corpo} }); }
    catch (x) { return JSON.stringify({ erro: String(x?.message || x) }); }
  };`, ctx);
  return entrada => JSON.parse(vm.runInContext(`${nome}(${JSON.stringify(JSON.stringify(entrada))})`, ctx));
}

export async function apostasDoDespejo(despejo) {
  const linhas = readline.createInterface({ input: fs.createReadStream(despejo, 'utf8'), crlfDelay: Infinity });
  for await (const linha of linhas) {
    if (!linha.startsWith('{"url":"/api/history/closed"')) continue;
    return JSON.parse(linha).body.fechados;
  }
  throw new Error('o despejo não tem GET /api/history/closed');
}

/// Grava `{id, entrada, saida|erro}` por linha e devolve quantos casos deram exceção no JS.
export function gravarCasos(saida, casos, chamar) {
  const fd = fs.openSync(saida, 'w');
  let erros = 0;
  for (const caso of casos) {
    // A entrada gravada é a que o JS viu (JSON.stringify tira `undefined`).
    const entrada = caso.entrada === undefined ? null : JSON.parse(JSON.stringify(caso.entrada));
    const r = chamar(entrada);
    if (r.erro) erros++;
    fs.writeSync(fd, JSON.stringify({ id: caso.id, entrada, ...r }) + '\n');
  }
  fs.closeSync(fd);
  return erros;
}

/// Troca espaços por outros que o `\s` do JS conhece (U+FEFF, U+00A0, U+2028...) e
/// mistura U+0085 e `\r`, que o Rust e o JS tratam de jeitos diferentes.
export function espacosDoJs(s) {
  if (typeof s !== 'string') return s;
  const trocas = ['﻿', ' \u0085', ' ', '\r', ' ', ' ', '  ', '\t', '　 '];
  let i = 0;
  return '﻿' + s.replace(/ /g, () => trocas[i++ % trocas.length]) + ' \u0085';
}
