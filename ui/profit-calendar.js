/* Calendar dates are civil dates in São Paulo, independent of the device zone. */
globalThis.BettyProfitCalendar = (() => {
  function validMonth(month) {
    return /^\d{4}-(0[1-9]|1[0-2])$/.test(month) && Number(month.slice(0, 4)) >= 1000;
  }
  function monthDays(month) {
    if (!validMonth(month)) return [];
    const [year, number] = month.split('-').map(Number);
    const offset = new Date(Date.UTC(year, number - 1, 1)).getUTCDay();
    const count = new Date(Date.UTC(year, number, 0)).getUTCDate();
    return Array.from({ length: Math.ceil((offset + count) / 7) * 7 }, (_, i) => {
      const day = i - offset + 1;
      return day < 1 || day > count ? null : `${month}-${String(day).padStart(2, '0')}`;
    });
  }
  function shiftMonth(month, step) {
    const [year, number] = month.split('-').map(Number);
    const date = new Date(Date.UTC(year, number - 1 + step, 1));
    const next = `${date.getUTCFullYear()}-${String(date.getUTCMonth() + 1).padStart(2, '0')}`;
    return validMonth(next) ? next : month;
  }
  function summary(days, unit = 'usd') {
    const total = { total: 0, lucroD: 0, volumeD: 0, lucroStake: 0, volumeStake: 0, wins: 0, losses: 0, green: 0, red: 0 };
    for (const day of days) {
      for (const key of ['total', 'lucroD', 'volumeD', 'lucroStake', 'volumeStake', 'wins', 'losses']) total[key] += Number(day[key]) || 0;
      const result = Number(day[unit === 'stake' ? 'lucroStake' : 'lucroD']) || 0;
      if (result > .0000001) total.green++;
      if (result < -.0000001) total.red++;
    }
    total.roi = total.volumeD > 0 ? total.lucroD / total.volumeD * 100 : 0;
    total.taxaAcerto = total.total ? total.wins / total.total * 100 : 0;
    return total;
  }
  return { validMonth, monthDays, shiftMonth, summary };
})();

// Tradução via i18n.js; sem ele (validação em Node), fica o português com os marcadores preenchidos.
function txCalendario(texto, vars) {
  if (globalThis.BettyI18n) return globalThis.BettyI18n.t(texto, vars);
  return vars ? String(texto).replace(/\{(\w+)\}/g, (marca, nome) => vars[nome] != null ? String(vars[nome]) : marca) : String(texto);
}

const profitCalendarState = {
  month: new Intl.DateTimeFormat('en-CA', { timeZone: 'America/Sao_Paulo', year: 'numeric', month: '2-digit' }).format(new Date()).slice(0, 7),
  house: 'todas', unit: 'usd', selected: null, data: null, key: '', version: -1, request: 0, detailRequest: 0, detailCollapsed: true
};
function profitCalendarKey() {
  return JSON.stringify([profitCalendarState.month,profitCalendarState.house,BETTY_USER?.id||'local',versaoLeituraHistorico(),chavePeriodoFechadasConfigurado()]);
}
let profitCalendarPickerYear;
function fecharSeletorMesProfit(focus = false) {
  const popup = document.getElementById('profitCalendarMonthPopup');
  if (!popup) return;
  popup.hidden = true; popup.classList.remove('open');
  const trigger = document.getElementById('profitCalendarMonthInput');
  trigger.setAttribute('aria-expanded', 'false');
  if (focus) trigger.focus();
}
function renderSeletorMesProfit() {
  const popup = document.getElementById('profitCalendarMonthPopup');
  const today = ymdBrasil(new Date()).slice(0,7);
  const months = Array.from({length:12},(_,i)=>{
    const month = `${profitCalendarPickerYear}-${String(i+1).padStart(2,'0')}`;
    const name = profitCalendarDate(month+'-01',{month:'short'}).replace('.','');
    return `<button type="button" class="day${month === today ? ' today' : ''}${month === profitCalendarState.month ? ' selected' : ''}" data-month="${month}" aria-label="${escapeHtml(profitCalendarDate(month+'-01',{month:'long',year:'numeric'}))}" aria-pressed="${month === profitCalendarState.month}">${name}</button>`;
  }).join('');
  popup.innerHTML = `<div class="date-popup-head"><span>${profitCalendarPickerYear}</span><div class="date-popup-nav"><button type="button" data-year-step="-1" aria-label="${txCalendario('Ano anterior')}" ${profitCalendarPickerYear<=1000?'disabled':''}>‹</button><button type="button" data-year-step="1" aria-label="${txCalendario('Próximo ano')}" ${profitCalendarPickerYear>=9999?'disabled':''}>›</button></div></div><div class="date-grid profit-calendar-month-grid">${months}</div><div class="date-popup-foot"><button type="button" data-current-month>${txCalendario('Este mês')}</button></div>`;
  popup.onclick = event => {
    event.stopPropagation();
    const button = event.target.closest('button'); if (!button) return;
    if (button.hasAttribute('data-year-step')) {
      const step = Number(button.dataset.yearStep);
      profitCalendarPickerYear = Math.max(1000,Math.min(9999,profitCalendarPickerYear+step));
      renderSeletorMesProfit(); popup.querySelector(`[data-year-step="${step}"]`).focus();
    } else {
      const month = button.hasAttribute('data-current-month') ? today : button.dataset.month;
      if (month) { fecharSeletorMesProfit(true); selecionarMesProfit(month); }
    }
  };
}
function toggleSeletorMesProfit(event) {
  event?.stopPropagation();
  const popup = document.getElementById('profitCalendarMonthPopup');
  if (!popup.hidden) { fecharSeletorMesProfit(); return; }
  profitCalendarPickerYear = Number(profitCalendarState.month.slice(0,4));
  renderSeletorMesProfit(); popup.hidden = false; popup.classList.add('open');
  const rect = document.getElementById('profitCalendarMonthInput').getBoundingClientRect();
  const width = popup.offsetWidth, height = popup.offsetHeight;
  popup.style.left = `${Math.max(8,Math.min(rect.left,window.innerWidth-width-8))}px`;
  popup.style.top = `${rect.bottom+height+8 > window.innerHeight-8 ? Math.max(8,rect.top-height-8) : rect.bottom+8}px`;
  popup.querySelector('.selected')?.focus();
}
if (typeof document !== 'undefined') {
  document.addEventListener('scroll', () => {
    const popup = document.getElementById('profitCalendarMonthPopup');
    if (popup && !popup.hidden) fecharSeletorMesProfit();
  }, { capture: true, passive: true });
  document.addEventListener('click',event=>{
    if (!event.target.closest('.profit-calendar-month-field')) fecharSeletorMesProfit();
  });
  document.addEventListener('keydown',event=>{
    const popup=document.getElementById('profitCalendarMonthPopup');
    if (!popup || popup.hidden) return;
    if (event.key==='Escape') { event.preventDefault(); fecharSeletorMesProfit(true); }
    const months=[...popup.querySelectorAll('[data-month]')],index=months.indexOf(document.activeElement);
    const step={ArrowLeft:-1,ArrowRight:1,ArrowUp:-3,ArrowDown:3}[event.key];
    if (index>=0 && step) { event.preventDefault(); months[Math.max(0,Math.min(11,index+step))].focus(); }
  });
}
function profitCalendarAmount(value, signed = false, compact = false) {
  const n = Math.abs(Number(value)) < .0000001 ? 0 : Number(value) || 0;
  const sign = n < 0 ? '−' : signed && n > 0 ? '+' : '';
  profitCalendarAmount.formatters ||= new Map();
  if (!profitCalendarAmount.formatters.has(compact)) profitCalendarAmount.formatters.set(compact,new Intl.NumberFormat(globalThis.BettyI18n?.locale || 'pt-BR', { minimumFractionDigits: compact ? 0 : 2, maximumFractionDigits: 2, ...(compact ? { notation: 'compact' } : {}) }));
  const number = profitCalendarAmount.formatters.get(compact).format(Math.abs(n));
  return sign + (profitCalendarState.unit === 'stake' ? number + ' u' : '$' + number);
}
function profitCalendarValue(row, volume = false) {
  return Number(row?.[volume ? (profitCalendarState.unit === 'stake' ? 'volumeStake' : 'volumeD') : (profitCalendarState.unit === 'stake' ? 'lucroStake' : 'lucroD')]) || 0;
}
function profitCalendarTone(value) { return value > .0000001 ? 'positive' : value < -.0000001 ? 'negative' : 'neutral'; }
function profitCalendarDate(day, options = { day: 'numeric', month: 'long' }) {
  profitCalendarDate.formatters ||= new Map();
  const key = JSON.stringify(options);
  if (!profitCalendarDate.formatters.has(key)) {
    if (profitCalendarDate.formatters.size >= 12) profitCalendarDate.formatters.delete(profitCalendarDate.formatters.keys().next().value);
    profitCalendarDate.formatters.set(key,new Intl.DateTimeFormat(globalThis.BettyI18n?.locale || 'pt-BR', { timeZone: 'UTC', ...options }));
  }
  return profitCalendarDate.formatters.get(key).format(new Date(day + 'T12:00:00Z'));
}
function mudarMesProfit(step) { selecionarMesProfit(BettyProfitCalendar.shiftMonth(profitCalendarState.month, step)); }
function selecionarMesProfit(month) {
  if (!BettyProfitCalendar.validMonth(month)) return;
  profitCalendarState.month = month;
  profitCalendarState.selected = null;
  renderProfitCalendar();
}
function irHojeProfit() {
  const today = ymdBrasil(new Date());
  profitCalendarState.month = today.slice(0, 7);
  profitCalendarState.selected = today;
  renderProfitCalendar();
}
function alterarCasaProfit(house) { profitCalendarState.house = house; renderProfitCalendar(); }
function alterarUnidadeProfit(unit) {
  profitCalendarState.unit = unit;
  pintarProfitCalendar();
  renderProfitCalendarDetail();
}
function profitCalendarStatus(message, retry = false) {
  const el = document.getElementById('profitCalendarStatus');
  el.textContent = message;
  if (retry) {
    const button = document.createElement('button');
    button.type = 'button'; button.className = 'calendar-control'; button.textContent = txCalendario('Tentar novamente');
    button.onclick = () => renderProfitCalendar(); el.append(' ', button);
  }
}
async function prepararProfitCalendar() {
  if(!dadosFinanceirosConfirmados())return null;
  const state=profitCalendarState,key=profitCalendarKey(),version=versaoLeituraHistorico(),scope=escopoConsultaPosicoes();
  if(state.data && state.key===key && state.version===version)return state.data;
  if(state.prepared?.key===key && state.prepared.scope===scope)return state.prepared.data;
  let pending=state.pending;
  if(!pending || pending.key!==key || pending.version!==version){
    const params=new URLSearchParams({month:state.month,platform:state.house});
    const promise=fetch(urlLeituraHistorico('/api/history/closed/calendar?'+params),{cache:'no-store'}).then(async response=>{
      if(!response.ok)throw new Error('Calendar HTTP '+response.status);
      const data=await response.json();if(!data?.ok || !Array.isArray(data.days))throw new Error('Invalid calendar');return data;
    });
    pending={key,version,promise};state.pending=pending;
  }
  try {
    const data=await pending.promise;
    if(scope!==escopoConsultaPosicoes() || key!==profitCalendarKey() || !resumoHistoricoPodeSerPublicado(version))return null;
    state.prepared={key,version,scope,data};return data;
  } catch(_){return null;}
  finally {if(state.pending===pending)state.pending=null;}
}
async function renderProfitCalendar(opcoes = {}) {
  fecharSeletorMesProfit();
  const state = profitCalendarState, request = ++state.request, key = profitCalendarKey();
  ++state.detailRequest;
  if (state.key !== key) { state.data = null; document.getElementById('profitCalendarDetail').replaceChildren(); }
  pintarProfitCalendar();
  if (!dadosFinanceirosConfirmados()) {
    profitCalendarStatus(txCalendario(historicoIndiceEmSincronizacao ? 'Histórico em atualização. Aguarde a sincronização.' : 'Aguardando a confirmação do histórico. Use Atualizar agora para sincronizar.'));
    return;
  }
  const version = versaoLeituraHistorico();
  if (!opcoes.forcar && state.data && state.key === key && state.version === version) {
    profitCalendarConfirmedStatus();
    renderProfitCalendarDetail();
    return;
  }
  profitCalendarStatus(txCalendario('Carregando resultados do mês…'));
  let pending;
  try {
    if(!opcoes.forcar && state.prepared?.key===key && state.prepared.version===version && state.prepared.scope===escopoConsultaPosicoes()) {
      state.pending={key,version,promise:Promise.resolve(state.prepared.data)};
    }
    const params = new URLSearchParams({ month: state.month, platform: state.house });
    pending = state.pending;
    if (!pending || pending.key !== key || pending.version !== version) {
      const promise = (async()=>{
        const response = await fetch(urlLeituraHistorico('/api/history/closed/calendar?' + params), { cache: 'no-store' });
        if (!response.ok) throw new Error('Calendar HTTP ' + response.status);
        const data = await response.json();
        if (!data?.ok || !Array.isArray(data.days)) throw new Error('Invalid calendar');
        return data;
      })();
      pending = {key,version,promise};
      state.pending = pending;
    }
    const data = await pending.promise;
    if (request !== state.request || viewAtual !== 'calendario') return;
    if (!resumoHistoricoPodeSerPublicado(version)) { profitCalendarStatus(txCalendario('Histórico em atualização. Aguarde a sincronização.')); return; }
    if (key !== profitCalendarKey()) return;
    state.data = data; state.key = key; state.version = version;
    if (!state.selected) {
      const today = ymdBrasil(new Date());
      state.selected = today.startsWith(state.month) ? today : data.days.at(-1)?.chave || state.month + '-01';
    }
    pintarProfitCalendar();
    profitCalendarConfirmedStatus();
    renderProfitCalendarDetail();
  } catch (error) {
    if (request !== state.request || viewAtual !== 'calendario') return;
    if (!resumoHistoricoPodeSerPublicado(version)) { profitCalendarStatus(txCalendario('Histórico em atualização. Aguarde a sincronização.')); return; }
    if (key !== profitCalendarKey()) return;
    profitCalendarStatus(txCalendario('Não foi possível carregar os resultados do mês. Verifique a conexão e tente novamente.'), true);
  } finally {
    if (state.pending === pending) state.pending = null;
  }
}
function profitCalendarConfirmedStatus() {
  const data=profitCalendarState.data,missing=Number(data?.missingDates)||0;
  profitCalendarStatus(txCalendario(data?.days.length ? 'Selecione um dia para ver as apostas e os indicadores.' : 'Nenhuma aposta fechada neste mês para a casa selecionada.') + (missing ? ' ' + txCalendario('{n} posição(ões) no histórico sem data de realização; fora do calendário.', { n: missing }) : ''));
}
function pintarProfitCalendar() {
  const state = profitCalendarState;
  const ready = state.key === profitCalendarKey() && state.data;
  document.getElementById('profitCalendarMonth').textContent = profitCalendarDate(state.month + '-01', { month: 'long', year: 'numeric' });
  document.getElementById('profitCalendarMonthInput').value = state.month;
  document.getElementById('profitCalendarMonthLabel').textContent = profitCalendarDate(state.month + '-01', {month:'long',year:'numeric'});
  const days = ready ? state.data.days : [];
  const summary = BettyProfitCalendar.summary(days, state.unit);
  const metric = (icon, color, label, value, sub = '', tone = '') => `<div class="stat-card calendar-metric"><div class="stat-icon ${color}" aria-hidden="true">${icon}</div><div class="stat-body"><div class="label">${label}</div><div class="value ${tone}">${ready ? value : '—'}</div>${sub ? `<div class="sub">${ready ? sub : txCalendario('Aguardando histórico')}</div>` : ''}</div></div>`;
  document.getElementById('profitCalendarSummary').innerHTML =
    metric(ICONS.trendUp, profitCalendarValue(summary) < 0 ? 'red' : 'green', txCalendario('Lucro do mês'), profitCalendarAmount(profitCalendarValue(summary), true), txCalendario('{positivos} dias positivos · {negativos} negativos', { positivos: summary.green, negativos: summary.red }), profitCalendarTone(profitCalendarValue(summary))) +
    metric(ICONS.percent, 'info', 'ROI', summary.roi.toFixed(2) + '%', txCalendario('Lucro / volume em US$')) +
    metric(ICONS.db, 'blue', txCalendario('Volume'), profitCalendarAmount(profitCalendarValue(summary, true)), txCalendario(summary.total === 1 ? '{n} aposta fechada' : '{n} apostas fechadas', { n: summary.total })) +
    metric(ICONS.target, 'purple', txCalendario('Taxa de acerto'), summary.taxaAcerto.toFixed(1) + '%', txCalendario('{ganhas} ganhas · {perdidas} perdidas', { ganhas: summary.wins, perdidas: summary.losses }));
  const byDay = new Map(days.map(day => [day.chave, day]));
  const today = ymdBrasil(new Date());
  const dates = BettyProfitCalendar.monthDays(state.month);
  let html = ['Dom', 'Seg', 'Ter', 'Qua', 'Qui', 'Sex', 'Sáb', 'Semana'].map((label, i) => `<div class="calendar-weekday${i === 7 ? ' calendar-week-total' : ''}">${txCalendario(label)}</div>`).join('');
  // Intensidade: dias com pelo menos metade do maior resultado do mês ficam mais fortes.
  const maiorDia = Math.max(0, ...dates.filter(Boolean).map(d => byDay.get(d)?.total ? Math.abs(Number(profitCalendarValue(byDay.get(d))) || 0) : 0));
  dates.forEach((day, i) => {
    if (!day) html += '<div class="calendar-blank" aria-hidden="true"></div>';
    else {
      const row = byDay.get(day), value = profitCalendarValue(row), forte = row?.total && maiorDia > 0 && Math.abs(Number(value) || 0) >= maiorDia / 2, tone = row?.total ? profitCalendarTone(value) + (forte ? ' is-strong' : '') : 'calendar-no-bets';
      const label = `${profitCalendarDate(day)}: ${ready ? row?.total ? txCalendario(row.total === 1 ? '{valor}, {n} aposta' : '{valor}, {n} apostas', { valor: profitCalendarAmount(value, true), n: row.total }) : txCalendario('sem apostas fechadas') : txCalendario('aguardando histórico')}`;
      html += `<button type="button" class="calendar-day ${tone}${day === today ? ' is-today' : ''}${day === state.selected ? ' is-selected' : ''}" data-day="${day}" aria-label="${escapeHtml(label)}" aria-pressed="${day === state.selected}" ${ready ? '' : 'disabled'}>
        <span class="calendar-day-number">${Number(day.slice(-2))}${day === today ? `<span class="calendar-today-label">${txCalendario('Hoje')}</span>` : ''}</span>
        <strong class="calendar-day-profit"><span class="calendar-full-amount">${ready ? row?.total ? profitCalendarAmount(value, true) : '—' : '…'}</span><span class="calendar-short-amount">${ready ? row?.total ? profitCalendarAmount(value, true, true) : '—' : '…'}</span></strong>
        <small>${ready && row?.total ? `${row.total}<span class="calendar-bets-label"> ${txCalendario(row.total === 1 ? 'aposta' : 'apostas')}</span>` : `<span class="calendar-bets-label">${txCalendario('Sem apostas')}</span>`}</small></button>`;
    }
    if (i % 7 === 6) {
      const week = BettyProfitCalendar.summary(dates.slice(i - 6, i + 1).map(date => byDay.get(date)).filter(Boolean), state.unit);
      html += `<div class="calendar-week-total calendar-week-summary"><span>${txCalendario('Semana {n}', { n: Math.floor(i / 7) + 1 })}</span><strong class="${profitCalendarTone(profitCalendarValue(week))}">${ready ? profitCalendarAmount(profitCalendarValue(week), true) : '—'}</strong><small>${ready ? txCalendario(week.total === 1 ? '{n} aposta' : '{n} apostas', { n: week.total }) : ''}</small></div>`;
    }
  });
  const grid = document.getElementById('profitCalendarGrid');
  grid.innerHTML = html;
  grid.querySelectorAll('[data-day]').forEach(button => button.onclick = () => {
    state.selected = button.dataset.day;
    // Preserve keyboard focus while changing the selected day.
    grid.querySelectorAll('[data-day]').forEach(day => { const selected = day.dataset.day === state.selected; day.classList.toggle('is-selected', selected); day.setAttribute('aria-pressed', String(selected)); });
    renderProfitCalendarDetail();
  });
}
async function renderProfitCalendarDetail(page = 1) {
  const state = profitCalendarState, request = ++state.detailRequest, key = profitCalendarKey(), day = state.selected;
  const el = document.getElementById('profitCalendarDetail');
  if (!day || state.key !== key || !state.data) { el.replaceChildren(); return; }
  const row = state.data.days.find(row => row.chave === day), pnl = profitCalendarValue(row);
  el.innerHTML = `<details class="calendar-day-details" ${state.detailCollapsed ? '' : 'open'}><summary class="calendar-detail-heading"><span class="calendar-detail-chevron" aria-hidden="true">›</span><div><h3>${profitCalendarDate(day, { weekday: 'long', day: 'numeric', month: 'long', year: 'numeric' })}</h3><p>${txCalendario(row?.total === 1 ? '{n} aposta fechada' : '{n} apostas fechadas', { n: row?.total || 0 })}${row?.total ? ' · ' + txCalendario('Volume {volume} · ROI {roi}% · Acerto {acerto}%', { volume: profitCalendarAmount(profitCalendarValue(row, true)), roi: Number(row.roi).toFixed(2), acerto: Number(row.taxaAcerto).toFixed(1) }) : ''}</p></div><span class="calendar-detail-toggle"><span class="calendar-collapse-label">${txCalendario('Recolher apostas')}</span><span class="calendar-expand-label">${txCalendario('Mostrar apostas')}</span></span><strong class="${profitCalendarTone(pnl)}">${profitCalendarAmount(pnl, true)}</strong></summary><div id="profitCalendarDayRows" aria-live="polite"></div></details>`;
  el.querySelector('details').ontoggle = event => { state.detailCollapsed = !event.currentTarget.open; };
  const rowsEl = document.getElementById('profitCalendarDayRows');
  if (!row?.total) { rowsEl.innerHTML = `<p class="calendar-detail-empty">${txCalendario('Nenhuma aposta fechada neste dia.')}</p>`; return; }
  if (!dadosFinanceirosConfirmados() || state.version !== versaoLeituraHistorico()) { rowsEl.textContent = txCalendario('Aguarde a confirmação do histórico para consultar as apostas.'); return; }
  rowsEl.textContent = txCalendario('Carregando apostas…');
  const version = versaoLeituraHistorico();
  try {
    const params = new URLSearchParams({ day, platform: state.house, page: String(page), pageSize: '10' });
    const data = await consultarPaginaFechadas(params);
    if (!data?.ok || !Array.isArray(data.fechados)) throw new Error('Invalid day');
    if (request !== state.detailRequest || key !== profitCalendarKey() || day !== state.selected || viewAtual !== 'calendario') return;
    if (!resumoHistoricoPodeSerPublicado(version)) { rowsEl.textContent = txCalendario('Histórico em atualização. Selecione o dia após a sincronização.'); return; }
    rowsEl.innerHTML = '<ul class="calendar-bet-list">' + data.fechados.map(bet => {
      const result = Number(bet.realizedPnl) || 0;
      const value = state.unit === 'stake' ? dolarParaUnidades(result, getTsPnL(bet), bet._platform) : result;
      const volume = state.unit === 'stake' ? dolarParaUnidades(getStake(bet), getTsPnL(bet), bet._platform) : getStake(bet);
      const original = tituloExibicao(bet) || '—';
      // Sem BettyI18n (validação em Node) o título fica em português.
      const title = globalThis.BettyI18n?.tTitulo(original) ?? original;
      const positions = BettyMarketTitles.positionColumns(original, bet.outcome, bet);
      const columns = globalThis.BettyI18n?.tColunas(positions) ?? positions;
      const sport = classificarAposta(bet);
      const platform = casaDaAposta(bet);
      const metadata = txCalendario('Stake {stake} · Odd {odd}', { stake: profitCalendarAmount(volume), odd: getOddNum(bet).toFixed(2) });
      return `<li><div class="calendar-bet-description" title="${escapeHtml(title + ' — ' + (globalThis.BettyI18n?.tTitulo(bet.outcome) || bet.outcome || '—') + ' · ' + metadata)}"><div class="calendar-bet-top"><span class="calendar-bet-game">${iconeCategoria(sport)}<span>${escapeHtml(columns.game)}</span></span><span class="calendar-bet-divider" aria-hidden="true">—</span><span class="calendar-bet-selection">${escapeHtml(columns.bet)}</span></div><div class="calendar-bet-title"><span class="calendar-bet-house calendar-house-${platform}">${NOMES_CASAS[platform]}</span><small class="calendar-bet-meta">${escapeHtml(metadata)}</small></div></div><strong class="${profitCalendarTone(value)}">${profitCalendarAmount(value, true)}</strong></li>`;
    }).join('') + '</ul>';
    const pages = Math.ceil(data.total / 10);
    if (pages > 1) {
      const pager = document.createElement('div'); pager.className = 'calendar-day-pager';
      const previous = document.createElement('button'), next = document.createElement('button');
      previous.type = next.type = 'button'; previous.className = next.className = 'calendar-control';
      previous.textContent = txCalendario('Anterior'); previous.disabled = page <= 1; previous.onclick = () => renderProfitCalendarDetail(page - 1);
      next.textContent = txCalendario('Próxima'); next.disabled = page >= pages; next.onclick = () => renderProfitCalendarDetail(page + 1);
      pager.append(previous, ` ${txCalendario('{pagina} de {total}', { pagina: page, total: pages })} `, next); rowsEl.append(pager);
    }
  } catch (error) {
    if (request !== state.detailRequest || viewAtual !== 'calendario') return;
    rowsEl.textContent = txCalendario('Não foi possível carregar as apostas deste dia.') + ' ';
    const retry = document.createElement('button'); retry.type = 'button'; retry.className = 'calendar-control'; retry.textContent = txCalendario('Tentar novamente'); retry.onclick = () => renderProfitCalendarDetail(page); rowsEl.append(retry);
  }
}
