// Ações declaradas no HTML sem script embutido (o CSP bloqueia `onclick=`).
//
// Um elemento declara `data-on-click="funcao('texto', 2, this.value, event)"`
// e este arquivo liga um listener no próprio elemento, como faria o atributo
// `onclick`. Sem eval: só chamadas de funções globais, com argumentos literais
// (texto entre aspas simples, número, true/false/null) ou `this`,
// `this.value` e `event`. Vários comandos podem ser separados por `;`.
(function () {
  'use strict';
  const TIPOS = ['click', 'change', 'input', 'keydown'];
  const ATRIBUTOS = TIPOS.map(t => 'data-on-' + t);
  const SELETOR = ATRIBUTOS.map(a => '[' + a + ']').join(',');
  const cache = new Map();
  const ligados = new WeakMap();

  function erro(texto, motivo) {
    return new Error('Ação inválida (' + motivo + '): ' + texto);
  }

  // Analisa `f(a, b); g()` em [{nome, args: [{tipo, valor}]}].
  function analisar(texto) {
    const em = cache.get(texto);
    if (em) return em;
    let i = 0;
    const espaco = () => { while (i < texto.length && /\s/.test(texto[i])) i++; };
    const identificador = () => {
      const m = /^[A-Za-z_$][\w$]*(?:\.[A-Za-z_$][\w$]*)?/.exec(texto.slice(i));
      if (!m) throw erro(texto, 'nome esperado na posição ' + i);
      i += m[0].length;
      return m[0];
    };
    const argumento = () => {
      espaco();
      const c = texto[i];
      if (c === "'") {
        let s = '';
        i++;
        while (i < texto.length && texto[i] !== "'") {
          if (texto[i] === '\\') i++;
          s += texto[i++];
        }
        if (texto[i] !== "'") throw erro(texto, 'texto sem fim');
        i++;
        return { tipo: 'literal', valor: s };
      }
      const num = /^-?\d+(?:\.\d+)?/.exec(texto.slice(i));
      if (num) { i += num[0].length; return { tipo: 'literal', valor: Number(num[0]) }; }
      const nome = identificador();
      if (nome === 'true' || nome === 'false') return { tipo: 'literal', valor: nome === 'true' };
      if (nome === 'null') return { tipo: 'literal', valor: null };
      if (nome === 'this' || nome === 'this.value' || nome === 'event') return { tipo: nome };
      throw erro(texto, 'argumento não suportado: ' + nome);
    };
    const comandos = [];
    while (true) {
      espaco();
      if (i >= texto.length) break;
      const nome = identificador();
      if (nome.includes('.')) throw erro(texto, 'só funções globais');
      espaco();
      if (texto[i] !== '(') throw erro(texto, '( esperado');
      i++;
      const args = [];
      espaco();
      if (texto[i] !== ')') {
        while (true) {
          args.push(argumento());
          espaco();
          if (texto[i] === ',') { i++; continue; }
          break;
        }
      }
      if (texto[i] !== ')') throw erro(texto, ') esperado');
      i++;
      comandos.push({ nome, args });
      espaco();
      if (texto[i] === ';') { i++; continue; }
      espaco();
      if (i < texto.length) throw erro(texto, 'sobra na posição ' + i);
    }
    cache.set(texto, comandos);
    return comandos;
  }

  function executar(el, evento, texto) {
    for (const { nome, args } of analisar(texto)) {
      const fn = window[nome];
      if (typeof fn !== 'function') throw new Error('Função de ação inexistente: ' + nome);
      fn(...args.map(a => a.tipo === 'literal' ? a.valor : a.tipo === 'this' ? el : a.tipo === 'this.value' ? el.value : evento));
    }
  }

  function ligar(el) {
    let tipos = ligados.get(el);
    if (!tipos) { tipos = new Set(); ligados.set(el, tipos); }
    for (const tipo of TIPOS) {
      if (tipos.has(tipo) || !el.hasAttribute('data-on-' + tipo)) continue;
      tipos.add(tipo);
      // Lê o atributo na hora do evento: trocar o valor troca a ação.
      el.addEventListener(tipo, function (evento) {
        const texto = el.getAttribute('data-on-' + tipo);
        if (texto) executar(el, evento, texto);
      });
    }
  }

  function ligarArvore(raiz) {
    if (raiz.nodeType !== 1) return;
    if (raiz.matches(SELETOR)) ligar(raiz);
    raiz.querySelectorAll(SELETOR).forEach(ligar);
  }

  ligarArvore(document.documentElement);
  new MutationObserver(mudancas => {
    for (const m of mudancas) {
      if (m.type === 'attributes') ligar(m.target);
      else m.addedNodes.forEach(ligarArvore);
    }
  }).observe(document.documentElement, { childList: true, subtree: true, attributes: true, attributeFilter: ATRIBUTOS });

  window.BettyAcoes = { analisar };
})();
