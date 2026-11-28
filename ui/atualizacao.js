// Aviso de versão nova: o backend compara a versão instalada com a última
// release do GitHub (`/api/atualizacao`). Havendo uma mais nova, aparece o
// botão "Atualizar para X" no topo de todas as telas; ele abre a confirmação,
// e "Atualizar agora" baixa e roda o instalador (o Betty fecha e reabre).
(function () {
  'use strict';
  const INTERVALO = 6 * 60 * 60 * 1000;
  let info = null;
  let foco = null;
  let instalando = false;

  const $ = id => document.getElementById(id);

  function formatarTamanho(bytes) {
    if (!Number.isFinite(bytes) || bytes <= 0) return '';
    return (bytes / 1048576).toLocaleString('pt-BR', { maximumFractionDigits: 1 }) + ' MB';
  }

  function formatarData(iso) {
    const d = new Date(iso);
    if (!iso || Number.isNaN(d.getTime())) return '';
    return d.toLocaleDateString('pt-BR', { day: '2-digit', month: 'short', year: 'numeric', timeZone: 'America/Sao_Paulo' });
  }

  // Notas da release em texto simples (o corpo vem em Markdown).
  function notasSimples(texto) {
    return String(texto || '')
      .replace(/\r/g, '')
      .replace(/^#{1,6}\s*/gm, '')
      .replace(/\*\*(.+?)\*\*/g, '$1')
      .replace(/`([^`]+)`/g, '$1')
      .replace(/\[([^\]]+)\]\([^)]*\)/g, '$1')
      .trim();
  }

  function renderizar() {
    const botao = $('btnAtualizacao');
    const versao = $('sidebarVersao');
    if (versao && info?.atual) {
      versao.textContent = 'Betty ' + info.atual;
      versao.title = info.disponivel ? 'Versão ' + info.ultima + ' disponível' : 'Você está na versão mais recente';
    }
    if (!botao) return;
    botao.hidden = !info?.disponivel;
    if (info?.disponivel) $('btnAtualizacaoVersao').textContent = info.ultima;
  }

  async function verificarAtualizacao(forcar) {
    try {
      const r = await fetch('/api/atualizacao' + (forcar ? '?forcar=1' : ''), { cache: 'no-store', headers: { Accept: 'application/json' } });
      if (!r.ok) return;
      info = await r.json();
      renderizar();
    } catch (_) { /* sem rede: tenta de novo no próximo intervalo */ }
  }

  function abrirAtualizacao() {
    if (!info?.disponivel) return;
    foco = document.activeElement;
    const tamanho = formatarTamanho(info.tamanho);
    const data = formatarData(info.publicada);
    $('atualizacaoTitulo').textContent = 'Betty ' + info.ultima + ' disponível';
    $('atualizacaoSub').textContent = 'Você está na versão ' + info.atual + '.'
      + (data ? ' Publicada em ' + data + '.' : '')
      + (tamanho ? ' Download de ' + tamanho + '.' : '');
    const notas = notasSimples(info.notas);
    $('atualizacaoNotas').textContent = notas;
    $('atualizacaoNotas').hidden = !notas;
    $('atualizacaoErro').textContent = '';
    const instalar = $('atualizacaoInstalar');
    instalar.disabled = false;
    instalar.textContent = 'Atualizar agora';
    const modal = $('atualizacaoModal');
    modal.classList.add('show');
    modal.setAttribute('aria-hidden', 'false');
    instalar.focus();
  }

  function fecharAtualizacao() {
    if (instalando) return;
    const modal = $('atualizacaoModal');
    modal.classList.remove('show');
    modal.setAttribute('aria-hidden', 'true');
    foco?.focus?.();
    foco = null;
  }

  async function instalarAtualizacao() {
    if (instalando) return;
    instalando = true;
    const botao = $('atualizacaoInstalar');
    botao.disabled = true;
    botao.textContent = 'Baixando…';
    $('atualizacaoErro').textContent = '';
    try {
      const r = await fetch('/api/atualizacao/instalar', { method: 'POST', cache: 'no-store' });
      const d = await r.json().catch(() => ({}));
      if (!r.ok || !d.ok) throw new Error(d.error || 'Não foi possível atualizar.');
      botao.textContent = 'Instalando… o Betty vai reabrir';
      // O backend fecha o app em seguida; nada mais a fazer aqui.
    } catch (e) {
      instalando = false;
      botao.disabled = false;
      botao.textContent = 'Tentar de novo';
      $('atualizacaoErro').textContent = (e && e.message) || 'Não foi possível atualizar.';
    }
  }

  $('atualizacaoModal')?.addEventListener('keydown', e => { if (e.key === 'Escape') fecharAtualizacao(); });
  $('atualizacaoModal')?.addEventListener('click', e => { if (e.target === e.currentTarget) fecharAtualizacao(); });

  window.abrirAtualizacao = abrirAtualizacao;
  window.fecharAtualizacao = fecharAtualizacao;
  window.instalarAtualizacao = instalarAtualizacao;

  // Alguns segundos depois da abertura, para não disputar com a carga inicial.
  setTimeout(() => verificarAtualizacao(false), 4000);
  setInterval(() => verificarAtualizacao(true), INTERVALO);
})();
