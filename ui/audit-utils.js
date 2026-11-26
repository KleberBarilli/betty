/* Pure helpers shared by the dashboard and the local server. No I/O on import. */
(() => {
  function timestampMs(value) {
    if (value == null || value === '') return null;
    const n = Number(value);
    const ms = Number.isFinite(n) ? (Math.abs(n) < 1e12 ? n * 1000 : n) : Date.parse(value);
    const d = new Date(ms);
    return Number.isFinite(ms) && d.getUTCFullYear() >= 2000 && d.getUTCFullYear() <= 2100 ? ms : null;
  }
  const isoDate = value => {
    const ms = timestampMs(value);
    return ms == null ? '' : new Date(ms).toISOString();
  };
  const csvDate = value => {
    const ms = timestampMs(value);
    return ms == null ? '' : new Date(ms).toLocaleString('pt-BR', { timeZone: 'America/Sao_Paulo', dateStyle: 'short', timeStyle: 'medium' });
  };
  async function collectPages(fetchPage, field, maxPages = 1000) {
    const out = [], seen = new Set();
    let cursor = '';
    for (let page = 0; page < maxPages; page++) {
      const data = await fetchPage(cursor);
      if (!Array.isArray(data?.[field])) throw new Error('Resposta de histórico inválida');
      out.push(...data[field]);
      cursor = String(data.cursor || '');
      if (!cursor) return out;
      if (seen.has(cursor)) throw new Error('Histórico incompleto: cursor repetido');
      seen.add(cursor);
    }
    throw new Error('Histórico incompleto: limite de paginação atingido');
  }
  function resolvedLoss(d) {
    return (d._platform || 'polymarket').toLowerCase() === 'polymarket'
      && d.redeemable === true && d.curPrice != null && Number(d.curPrice) === 0;
  }
  function selectExport({ open, closed, scope, housePeriod, statsFilter }) {
    if (!['all', 'periods', 'stats'].includes(scope)) throw new Error('Escopo inválido');
    return {
      open: scope === 'stats' ? [] : open,
      closed: scope === 'all' ? closed : closed.filter(d => housePeriod(d) && (scope !== 'stats' || statsFilter(d))),
    };
  }
  function pnlTimestamp(d, marketResolution) {
    const confirmed=timestampMs(d?._confirmedClosedTs);
    if(confirmed)return confirmed;
    if (['predict', 'sxbet'].includes(d?._platform)) return timestampMs(d._realizedAt) || 0;
    if (String(d?._platform || 'polymarket').toLowerCase() === 'kalshi') {
      return timestampMs(d?._settledTime || d?.settled_time || d?.settlement_time)
        || timestampMs(d?._saleTs || d?._lastTradeTs) || 0;
    }
    const resolution = timestampMs(d?._resolvedTime || marketResolution);
    // An unredeemed resolved loss still carries its purchase timestamp.
    if (resolvedLoss(d)) return resolution || 0;
    const closed = timestampMs(d?._saleTs || d?._lastTradeTs || d?.timestamp);
    // Sale before resolution: use the sale. Redemption later: use resolution.
    return closed && resolution ? Math.min(closed,resolution) : closed || resolution || 0;
  }
  function protectClosedDate(oldRow,newRow) {
    const row={...newRow};
    // The server derives the date; a new API/client timestamp cannot replace
    // the canonical confirmation merely by carrying a confirmation field.
    delete row._confirmedClosedTs;
    const candidate=pnlTimestamp(row), previous=pnlTimestamp(oldRow||{});
    const kalshi=String(row._platform||oldRow?._platform||'polymarket').toLowerCase()==='kalshi';
    const evidence=row._saleEvidence;
    const saleTs=timestampMs(evidence?.timestamp);
    // Actual SELL evidence is required, not merely changed amounts. It can
    // arrive after the financial refresh because the activity cache is shared.
    const newSale=!kalshi && evidence?.transactionHash &&
      String(evidence.asset||'')===String(row.asset||'') && saleTs>previous &&
      evidence.transactionHash!==oldRow?._saleEvidence?.transactionHash;
    // A newly recovered official resolution can correct an earlier provisional
    // API close/redemption date. Missing dates are never frozen.
    const recoveredResolution=!kalshi && !timestampMs(oldRow?._resolvedTime) &&
      timestampMs(row._resolvedTime) && candidate>0 && candidate<previous;
    let confirmed=previous||candidate;
    if(newSale) {
      const resolution=timestampMs(row._resolvedTime);
      confirmed=resolution ? Math.min(saleTs,resolution) : saleTs;
    } else if(recoveredResolution) confirmed=candidate;
    if(kalshi && timestampMs(oldRow?._settledTime))row._settledTime=oldRow._settledTime;
    if(confirmed>0)row._confirmedClosedTs=confirmed;
    return row;
  }
  globalThis.BettyAudit = Object.freeze({ timestampMs, isoDate, csvDate, collectPages, resolvedLoss, selectExport, pnlTimestamp, protectClosedDate });
})();
