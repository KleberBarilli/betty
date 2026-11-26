(function(root) {
  'use strict';
  const bounds = [1.01, 1.10, 1.20, 1.40, 1.60, 1.80, 2.00, 3.00, 5.00];
  // Keep persisted IDs stable while moving each upper boundary into the preceding range.
  const ranges = Object.freeze(bounds.map((oldMin, i) => {
    const min = i === 0 ? oldMin : (Math.round(oldMin * 100) + 1) / 100;
    const max = bounds[i + 1] ?? null;
    return Object.freeze({ id:String(oldMin), min, max,
      label:max == null ? `${min.toFixed(2)} ou mais` : `${min.toFixed(2)} – ${max.toFixed(2)}` });
  }));
  const round = odd => Number.isFinite(odd) && odd > 0 ? Math.round(odd * 100 + 1e-9) / 100 : 0;
  const get = id => ranges.find(r => r.id === String(id)) || null;
  const matches = (odd, id) => {
    const range = get(id);
    const value = round(odd);
    return !range || (value >= range.min && (range.max == null || value <= range.max));
  };
  const labelFor = odd => ranges.find(r => matches(odd,r.id))?.label
    || (Number.isFinite(odd) && odd > 0 ? 'Abaixo de 1.01' : 'Odd não disponível');
  root.BettyOddsRanges = Object.freeze({ ranges, get, matches, labelFor, round });
})(globalThis);
