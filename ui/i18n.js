// Idioma da interface: português do Brasil (padrão) ou inglês.
//
// O texto em português no código é a própria chave: `tx("Visão geral")` devolve
// o texto como está em pt-BR e a tradução de `i18n-en.js` em inglês. Sem
// tradução registrada, fica o português, então nada some da tela.
//
// Textos com partes variáveis usam marcadores: `tx("{n} posições", { n })`.
// Plural: escolher a chave no código (`n === 1 ? "{n} posição" : "{n} posições"`).
//
// O HTML estático (`dashboard.html`) é traduzido na carga por `traduzirDom`,
// que troca nós de texto e os atributos `aria-label`, `title`, `placeholder` e
// `alt` cujo conteúdo (sem os espaços das pontas) tem tradução.
//
// Só apresentação: nunca traduzir valores comparados na lógica, guardados no
// banco/localStorage ou mandados ao backend.
(function () {
  'use strict';
  const CHAVE = 'betty:idioma';
  const IDIOMAS = { 'pt-BR': 'Português (Brasil)', en: 'English' };
  const LOCALES = { 'pt-BR': 'pt-BR', en: 'en-US' };

  let idioma = 'pt-BR';
  try {
    const salvo = localStorage.getItem(CHAVE);
    if (salvo && IDIOMAS[salvo]) idioma = salvo;
  } catch (_) {}

  const dicionarios = { en: new Map() };
  // Mensagens com partes variáveis vindas do backend: [regex, substituição].
  const padroes = { en: [] };

  function preencher(texto, vars) {
    if (!vars) return texto;
    return texto.replace(/\{(\w+)\}/g, (marca, nome) =>
      Object.prototype.hasOwnProperty.call(vars, nome) && vars[nome] != null ? String(vars[nome]) : marca);
  }

  function traducao(texto) {
    if (idioma === 'pt-BR') return null;
    const dic = dicionarios[idioma];
    return dic && dic.has(texto) ? dic.get(texto) : null;
  }

  // Um mesmo texto pt com dois sentidos em inglês leva um contexto na chave:
  // `tx("atualizacao::Atualizar agora")` mostra "Atualizar agora" em pt-BR.
  const CONTEXTO = /^[\w-]+::/;

  function t(texto, vars) {
    const s = texto == null ? '' : String(texto);
    const tr = traducao(s);
    return preencher(tr == null ? s.replace(CONTEXTO, '') : tr, vars);
  }

  // Mensagem pronta vinda de fora (backend, erro de API): tenta o texto
  // exato, depois os padrões registrados; sem nada, devolve como veio.
  function tMsg(msg) {
    if (msg == null) return '';
    const s = String(msg);
    if (idioma === 'pt-BR') return s;
    const tr = traducao(s);
    if (tr != null) return tr;
    for (const [re, sub] of padroes[idioma] || []) {
      if (re.test(s)) return s.replace(re, sub);
    }
    return s;
  }

  // Títulos de mercado: o Betty monta alguns trechos em português ("Empate",
  // "Handicap de sets:"). O título gravado e usado na lógica (índice, busca,
  // categorias, "Por Mercado") continua assim; só a exibição troca de idioma.
  // `palavra` casa a expressão inteira (acentos contam como letra).
  const palavra = (fonte, flags = 'gu') => new RegExp('(?<![\\p{L}])(?:' + fonte + ')(?![\\p{L}])', flags);
  const ordinalEn = n => n + (['th', 'st', 'nd', 'rd'][Number(n)] || 'th');
  const METODOS = { nocaute: 'knockout', 'finalização': 'submission', 'decisão': 'decision' };
  // A ordem importa: expressões longas antes das palavras que elas contêm.
  const TRECHOS = [
    [/^Posição$/g, 'Position'],
    [palavra('Confronto indisponível'), 'Matchup unavailable'],
    [palavra('Mercado não confirmado(?=:)'), 'Unconfirmed market'],
    [palavra('Seleção(?=:)'), 'Selection'],
    [/ — Sim(?= — |$)/g, ' — Yes'],
    [/ — Não(?= — |$)/g, ' — No'],
    [palavra('Ambos os lados'), 'Both sides'],
    [palavra('Qualquer equipe'), 'Either team'],
    [palavra('Nenhuma equipe'), 'Neither team'],
    [palavra('Placar exato'), 'Exact score'],
    [palavra('(Total|Escanteios) (ímpar|par)'), (_, base, p) => (base === 'Total' ? 'Total' : 'Corners') + (p === 'par' ? ' even' : ' odd')],
    [palavra('Handicap de mapas(?=:)'), 'Map handicap'],
    [palavra('Handicap de sets(?=:)'), 'Set handicap'],
    [palavra('Handicap de games(?=:)'), 'Game handicap'],
    [/([1-4])[º°] quarto(?!\p{L})/gu, (_, n) => ordinalEn(n) + ' Quarter'],
    [/([1-3])[º°] per[ií]odo(?!\p{L})/gu, (_, n) => ordinalEn(n) + ' Period'],
    [palavra('Vitória por (nocaute|finalização|decisão|KO/TKO|TKO|KO)'), (_, m) => 'Win by ' + (METODOS[m] || m)],
    [palavra('Pódio'), 'Podium'],
    [/(^| - )Torneio (?=.+ ML(?: No)?$)/g, '$1Tournament '],
    [palavra('Empate'), 'Draw'],
    [palavra('gols \\+ assistências'), 'goals + assists'],
    [palavra('Assistências'), 'Assists'],
    [palavra('assistências'), 'assists'],
    [palavra('Chutes no alvo'), 'Shots on target'],
    [palavra('chutes no alvo'), 'shots on target'],
    [palavra('chutes'), 'shots'],
    [palavra('cestas de 3'), '3-pointers'],
    [palavra('Escanteios'), 'Corners'],
    [palavra('escanteios'), 'corners'],
    [palavra('Rebotes'), 'Rebounds'],
    [palavra('rebotes'), 'rebounds'],
    [palavra('gols'), 'goals'],
    [palavra('pontos'), 'points'],
    [palavra('defesas'), 'saves'],
    [palavra('roubos'), 'steals'],
    [palavra('bloqueios'), 'blocks'],
    [palavra('mapas'), 'maps']
  ];

  function tTitulo(titulo) {
    if (titulo == null) return '';
    const s = String(titulo);
    if (idioma === 'pt-BR') return s;
    return TRECHOS.reduce((texto, [re, sub]) => texto.replace(re, sub), s);
  }

  // Colunas Jogo/Aposta (positionColumns roda sobre o título em português).
  function tColunas(colunas) {
    return { game: tTitulo(colunas.game), bet: tTitulo(colunas.bet) };
  }

  // Busca: o índice guarda o título em português. Devolve o termo digitado em
  // inglês reescrito com as palavras do índice ("corners" → "escanteios"), ou
  // '' quando não há o que trocar.
  const BUSCA = [
    ['unconfirmed market', 'mercado não confirmado'], ['either team', 'qualquer equipe'], ['neither team', 'nenhuma equipe'],
    ['both sides', 'ambos os lados'], ['exact score', 'placar exato'], ['map handicap', 'handicap de mapas'],
    ['set handicap', 'handicap de sets'], ['game handicap', 'handicap de games'], ['win by', 'vitória por'],
    ['knockout', 'nocaute'], ['submission', 'finalização'], ['decision', 'decisão'], ['podium', 'pódio'],
    ['tournament', 'torneio'], ['draw', 'empate'], ['shots on target', 'chutes no alvo'], ['shots', 'chutes'],
    ['assists', 'assistências'], ['assist', 'assistência'], ['3-pointers?', 'cestas de 3'], ['corners', 'escanteios'],
    ['corner', 'escanteio'], ['rebounds', 'rebotes'], ['rebound', 'rebote'], ['goals', 'gols'], ['goal', 'gol'],
    ['points', 'pontos'], ['point', 'ponto'], ['saves', 'defesas'], ['steals', 'roubos'], ['blocks', 'bloqueios'],
    ['maps', 'mapas'], ['selection', 'seleção']
  ].map(([en, pt]) => [palavra(en), pt]);

  function termoNoIndice(termo) {
    const s = String(termo || '').toLowerCase();
    if (idioma === 'pt-BR') return '';
    const trocado = BUSCA.reduce((texto, [re, pt]) => texto.replace(re, pt), s);
    return trocado === s ? '' : trocado;
  }

  // O termo (já em minúsculas) casa com o título gravado ou com o exibido.
  function tituloContem(titulo, termo) {
    return String(titulo).toLowerCase().includes(termo) || tTitulo(titulo).toLowerCase().includes(termo);
  }

  // Rótulos compostos do backend ("Over gols — Um time — Partida inteira"):
  // cada pedaço é traduzido sozinho.
  function tRotulo(rotulo) {
    if (rotulo == null) return '';
    const s = String(rotulo);
    if (idioma === 'pt-BR') return s;
    // "Outros — Mercado não confirmado: …" carrega o título dentro do rótulo.
    if (s.includes('Mercado não confirmado:')) return tTitulo(s.replace(/^Outros — /, t('Outros') + ' — '));
    const inteiro = tMsg(s);
    return inteiro !== s ? inteiro : s.split(' — ').map(tMsg).join(' — ');
  }

  const ATRIBUTOS = ['aria-label', 'title', 'placeholder', 'alt'];

  function traduzirTextoNo(no) {
    const bruto = no.nodeValue;
    const limpo = bruto.trim();
    if (!limpo) return;
    const tr = traducao(limpo);
    if (tr == null) return;
    const inicio = bruto.slice(0, bruto.indexOf(limpo));
    const fim = bruto.slice(bruto.indexOf(limpo) + limpo.length);
    no.nodeValue = inicio + tr + fim;
  }

  function traduzirElemento(el) {
    // `data-i18n="contexto::Texto"`: chave com contexto para o texto do elemento.
    const chave = el.getAttribute('data-i18n');
    if (chave) {
      const tr = traducao(chave);
      if (tr != null) el.textContent = tr;
    }
    for (const attr of ATRIBUTOS) {
      const v = el.getAttribute(attr);
      if (!v) continue;
      const tr = traducao(v.trim());
      if (tr != null) el.setAttribute(attr, tr);
    }
    if (el.tagName === 'META' && el.getAttribute('name') === 'description') {
      const tr = traducao((el.getAttribute('content') || '').trim());
      if (tr != null) el.setAttribute('content', tr);
    }
  }

  function traduzirDom(raiz) {
    if (idioma === 'pt-BR' || !raiz) return;
    if (raiz.nodeType === Node.ELEMENT_NODE) traduzirElemento(raiz);
    const andar = document.createTreeWalker(raiz, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT, {
      acceptNode(no) {
        const pai = no.nodeType === Node.TEXT_NODE ? no.parentNode : no;
        if (pai && (pai.nodeName === 'SCRIPT' || pai.nodeName === 'STYLE')) return NodeFilter.FILTER_REJECT;
        return NodeFilter.FILTER_ACCEPT;
      }
    });
    let no;
    while ((no = andar.nextNode())) {
      if (no.nodeType === Node.TEXT_NODE) traduzirTextoNo(no);
      else traduzirElemento(no);
    }
  }

  // `i18n-en.js` chama ao carregar; a página (já parseada, os scripts ficam no
  // fim do body) é traduzida na hora, antes de `dashboard.js` rodar.
  function registrar(codigo, entradas, listaPadroes) {
    const dic = dicionarios[codigo] || (dicionarios[codigo] = new Map());
    for (const [pt, tr] of Object.entries(entradas || {})) {
      if (typeof tr === 'string' && tr) dic.set(pt, tr);
    }
    if (Array.isArray(listaPadroes)) (padroes[codigo] || (padroes[codigo] = [])).push(...listaPadroes);
    if (codigo === idioma) {
      document.documentElement.lang = idioma;
      traduzirDom(document.head);
      traduzirDom(document.body);
      if (document.title) document.title = t(document.title);
    }
  }

  // Datas para exibição (nunca para chaves ou parse): dia/mês no pt-BR,
  // mês/dia no inglês americano.
  function dataCurta(dd, mm) {
    const d = String(dd).padStart(2, '0');
    const m = String(mm).padStart(2, '0');
    return idioma === 'en' ? `${m}/${d}` : `${d}/${m}`;
  }
  function dataCompleta(dd, mm, aaaa) {
    return `${dataCurta(dd, mm)}/${aaaa}`;
  }
  // Texto "aaaa-mm-dd" (ou o começo de um ISO) para exibição.
  function dataYmd(ymd) {
    const m = /^(\d{4})-(\d{2})-(\d{2})/.exec(String(ymd || ''));
    return m ? dataCompleta(m[3], m[2], m[1]) : String(ymd || '');
  }

  function trocarIdioma(codigo) {
    if (!IDIOMAS[codigo] || codigo === idioma) return;
    try { localStorage.setItem(CHAVE, codigo); } catch (_) {}
    // Recarregar é o jeito seguro de todo texto já desenhado trocar de idioma;
    // os dados voltam do cache local na hora.
    location.reload();
  }

  globalThis.BettyI18n = {
    get idioma() { return idioma; },
    get locale() { return LOCALES[idioma]; },
    idiomas: IDIOMAS,
    t,
    tMsg,
    tRotulo,
    tTitulo,
    tColunas,
    termoNoIndice,
    tituloContem,
    registrar,
    traduzirDom,
    dataCurta,
    dataCompleta,
    dataYmd,
    trocarIdioma
  };
  globalThis.tx = t;
  globalThis.trocarIdioma = trocarIdioma;
  document.documentElement.lang = idioma;
  const seletor = document.getElementById('cfgIdioma');
  if (seletor) seletor.value = idioma;
})();
