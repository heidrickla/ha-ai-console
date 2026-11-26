(function () {
'use strict';
const NS = 'http://www.w3.org/2000/svg';
const FEEDS = ['quota', 'meter', 'routes', 'router', 'cloud', 'health', 'harness'];
const FEED_LABEL = { quota: 'quota', meter: 'meter', routes: 'routes', router: 'router', cloud: 'cloud', health: 'hosts', harness: 'harness' };
const CALLERS = ['voice', 'background', 'delegate'];
const WINDOWS = { '5m': 300, '1h': 3600, '24h': 86400 };
const POLL_MS = 15000;
const FETCH_MS = 10000;
const DAY = 86400000;
const S = {
  feeds: {}, raw: '', skew: 0, fetchedAt: 0, tab: 'usage', win: '1h', trend: 'day',
  filters: { model: '', label: '', status: '' }, offline: null, loaded: false, rendered: false,
};

/* ---------- small helpers ---------- */
function fill(e, attrs, kids) {
  for (const k in attrs || {}) {
    const v = attrs[k];
    if (v == null || v === false) continue;
    if (k === 'class') e.setAttribute('class', v);
    else if (k.startsWith('on')) e.addEventListener(k.slice(2), v);
    else e.setAttribute(k, v === true ? '' : String(v));
  }
  for (const kid of kids.flat(Infinity)) {
    if (kid == null || kid === false) continue;
    e.append(kid instanceof Node ? kid : document.createTextNode(String(kid)));
  }
  return e;
}
const h = (tag, attrs, ...kids) => fill(document.createElement(tag), attrs, kids);
const s = (tag, attrs, ...kids) => fill(document.createElementNS(NS, tag), attrs, kids);
const $ = (id) => document.getElementById(id);
const put = (parent, ...kids) => fill(parent, null, kids);
const clamp = (x, a, b) => Math.min(b, Math.max(a, x));
const isNum = (x) => typeof x === 'number' && isFinite(x);
const nowMs = () => Date.now() + S.skew;
const p2 = (n) => String(n).padStart(2, '0');

function trim1(x) { return (x < 100 ? x.toFixed(1) : String(Math.round(x))).replace(/\.0$/, ''); }
function fmtN(n) {
  if (!isNum(n)) return '–';
  const a = Math.abs(n);
  if (a >= 1e9) return trim1(n / 1e9) + 'B';
  if (a >= 1e6) return trim1(n / 1e6) + 'M';
  if (a >= 1e4) return trim1(n / 1e3) + 'K';
  return Math.round(n).toLocaleString('en-US');
}
function fmtUsd(n) {
  if (!isNum(n)) return '–';
  if (n >= 100) return '$' + Math.round(n).toLocaleString('en-US');
  return '$' + n.toFixed(n >= 1 || n === 0 ? 2 : n < 0.01 ? 4 : 3);
}
function fmtDur(sec) {
  sec = Math.max(0, Math.round(sec));
  const d = Math.floor(sec / 86400), hr = Math.floor(sec % 86400 / 3600), m = Math.floor(sec % 3600 / 60);
  if (d) return d + 'd ' + hr + 'h';
  if (hr) return hr + 'h ' + p2(m) + 'm';
  if (m) return m + 'm ' + p2(sec % 60) + 's';
  return sec + 's';
}
const fmtMs = (ms) => !isNum(ms) ? '–' : ms < 1000 ? Math.round(ms) + ' ms' : (ms / 1000).toFixed(1) + ' s';
const fmtPct = (x) => isNum(x) ? (Math.round(x * 10) / 10) + '%' : '–';
function localStamp(ms) {
  const d = new Date(ms);
  return p2(d.getMonth() + 1) + '-' + p2(d.getDate()) + ' ' + p2(d.getHours()) + ':' + p2(d.getMinutes()) + ':' + p2(d.getSeconds());
}
const tms = (iso) => Date.parse(iso);

function store(key, val) {
  try {
    if (val === undefined) return localStorage.getItem('ai-console.' + key);
    localStorage.setItem('ai-console.' + key, val);
  } catch (e) { /* storage can be blocked; the page works without it */ }
  return null;
}

/* A list collapsed to its newest rows; the choice is kept per list. */
const SHORT = 10;
const expander = (key, redraw) => {
  const btn = h('button', { type: 'button', class: 'btn', 'data-expand': key, onclick: () => { store('open.' + key, store('open.' + key) === '1' ? '0' : '1'); redraw(); } });
  return {
    btn,
    rows: (all) => {
      const open = store('open.' + key) === '1';
      btn.hidden = all.length <= SHORT;
      btn.setAttribute('aria-expanded', String(open));
      btn.textContent = open ? 'Show newest ' + SHORT : 'Show all ' + all.length;
      return open ? all : all.slice(0, SHORT);
    },
  };
};

/* Color follows the entity: the first seven names seen in a namespace keep slots 1 to 7, later ones fold into Other.
   A Map keeps a feed string such as "constructor" or "__proto__" from meaning anything but a name. */
const slotMaps = new Map();
function slot(ns, name) {
  let m = slotMaps.get(ns);
  if (!m) {
    m = new Map();
    try {
      const saved = JSON.parse(store('slots.' + ns) || '[]');
      if (Array.isArray(saved)) for (const n of saved) if (typeof n === 'string' && !m.has(n)) m.set(n, m.size);
    } catch (e) { /* a damaged stored list starts a new one */ }
    slotMaps.set(ns, m);
  }
  if (!m.has(name)) {
    m.set(name, m.size);
    store('slots.' + ns, JSON.stringify([...m.keys()]));
  }
  const i = m.get(name);
  return i < 7 ? 'var(--s' + (i + 1) + ')' : 'var(--other)';
}
const own = (o, k) => (o && typeof o === 'object' && Object.hasOwn(o, k) ? o[k] : undefined);
const dict = () => Object.create(null);

/* One card or section that throws shows its own error and leaves the rest of the view drawn. */
function errorCard(label, e) {
  return h('div', { class: 'card note bad', 'data-error': label }, '✕ ' + label + ' could not be drawn: ' + String((e && e.message) || e));
}
function guard(label, fn) {
  try { return fn(); } catch (e) { return errorCard(label, e); }
}

function feedData(name) { const f = S.feeds[name]; return f && f.present ? f.data : null; }
function feedAge(name) { const f = S.feeds[name]; return f && f.present ? f.age_s + (nowMs() - S.fetchedAt) / 1000 : null; }

/* ---------- tooltip: one element; hover and keyboard focus show the same rows ---------- */
function tipShow(evt, title, rows) {
  const t = $('tip');
  t.replaceChildren(
    title ? h('div', { class: 't' }, title) : null,
    rows.map((r) => h('div', { class: 'r' }, h('span', null, r.color ? h('i', { class: 'k', style: 'background:' + r.color }) : null, r.label), h('b', null, r.value))));
  t.hidden = false;
  let x, y;
  if (evt.clientX != null && evt.type !== 'focus') { x = evt.clientX; y = evt.clientY; }
  else { const b = evt.target.getBoundingClientRect(); x = b.left + b.width / 2; y = b.top; }
  const w = t.offsetWidth || 200, hh = t.offsetHeight || 60;
  t.style.left = clamp(x + 12, 4, Math.max(4, innerWidth - w - 4)) + 'px';
  t.style.top = clamp(y - hh - 10 < 4 ? y + 16 : y - hh - 10, 4, Math.max(4, innerHeight - hh - 4)) + 'px';
}
function tipHide() { $('tip').hidden = true; }
function bindTip(el, fn) {
  const show = (e) => tipShow(e, ...fn());
  el.addEventListener('pointerenter', show);
  el.addEventListener('pointermove', show);
  el.addEventListener('focus', show);
  el.addEventListener('pointerleave', tipHide);
  el.addEventListener('blur', tipHide);
  return el;
}

/* ---------- shared pieces ---------- */
function noData(feed, what) {
  const f = S.feeds[feed];
  const age = feedAge(feed);
  return h('div', { class: 'nodata', 'data-nodata': feed },
    h('b', null, 'No data yet. '), what || '',
    ' The ', FEED_LABEL[feed] || feed, ' feed ', age == null ? 'has not been received.' : 'was last read ' + fmtDur(age) + ' ago.',
    f && f.error ? h('div', { class: 'sub' }, 'Last error: ' + f.error) : null);
}
function feedNote(feed, text) {
  const f = S.feeds[feed];
  const age = feedAge(feed);
  return h('p', { class: 'note', 'data-nodata': feed }, 'No data yet: ', text, ' (', FEED_LABEL[feed] || feed, ' feed ', age == null ? 'never received' : fmtDur(age) + ' old', f && f.error ? '; last error: ' + f.error : '', ').');
}
function winSelect() {
  return h('div', { class: 'filters', role: 'group', 'aria-label': 'Window' },
    Object.keys(WINDOWS).map((k) => h('button', { type: 'button', class: 'btn', 'data-win': k, 'aria-pressed': S.win === k, onclick: () => { S.win = k; render(); } },
      k === '5m' ? '5 min' : k === '1h' ? '1 h' : '24 h')));
}
function legendItem(color, text) { return h('span', null, h('i', { style: 'background:' + color }), text); }

function meterBar(pct, severity, tick) {
  return h('div', { class: 'meter', role: 'meter', 'aria-valuemin': 0, 'aria-valuemax': 100, 'aria-valuenow': Math.round(pct) },
    h('div', { class: 'fill' + (severity ? ' ' + severity : ''), style: 'width:' + clamp(pct, 0, 100) + '%' }),
    tick != null ? h('div', { class: 'tick', style: 'left:' + clamp(tick, 0, 100) + '%', title: 'Share of the window elapsed' }) : null);
}

/* ---------- header ---------- */
function renderHeader() {
  $('ages').replaceChildren(...FEEDS.map((n) => {
    const f = S.feeds[n];
    const age = feedAge(n);
    const skew = f && f.skew_s ? f.skew_s : 0;
    const bad = (f && f.error && n !== 'quota' && n !== 'meter' && n !== 'routes') || skew;
    return h('span', { class: bad ? 'bad' : '', 'data-feed': n }, FEED_LABEL[n] + ' ',
      h('b', { 'data-age': n }, age == null ? 'no data' : fmtDur(age)), skew ? ' (clock skew: dated ' + fmtDur(skew) + ' ahead)' : bad ? ' (error)' : '');
  }));
  const b = $('budget');
  const m = feedData('meter');
  if (!m) { b.replaceChildren(h('div', { class: 'line' }, 'Cycle budget: no data yet')); return; }
  const c = m.cycle, total = c.budget_usd || 0;
  const spent = clamp(c.spent_usd, 0, total || c.spent_usd), reserved = clamp(c.reserved_usd, 0, Math.max(0, total - spent));
  const d = (iso) => new Date(tms(iso)).toLocaleDateString('en-US', { month: 'short', day: 'numeric', timeZone: 'UTC' });
  b.replaceChildren(
    h('div', { class: 'line' },
      h('span', null, 'Cycle ' + d(c.start) + ' to ' + d(c.end)),
      h('span', null, h('i', { class: 'sw', style: 'background:var(--s1)' }), 'spent ' + fmtUsd(c.spent_usd)),
      h('span', null, h('i', { class: 'sw', style: 'background:var(--s4)' }), 'reserved ' + fmtUsd(c.reserved_usd)),
      h('span', null, 'budget ' + fmtUsd(c.budget_usd))),
    total ? h('div', { class: 'seg', role: 'img', 'aria-label': 'Budget: ' + fmtUsd(c.spent_usd) + ' spent, ' + fmtUsd(c.reserved_usd) + ' reserved of ' + fmtUsd(total) },
      h('span', { style: 'width:' + (spent / total * 100) + '%;background:var(--s1)' }),
      h('span', { style: 'width:' + (reserved / total * 100) + '%;background:var(--s4)' })) : null);
}

/* ---------- usage ---------- */
function sparkline(values) {
  const W = 110, H = 34, pad = 5;
  const max = Math.max(...values, 0), n = values.length;
  const x = (i) => pad + (n < 2 ? 0 : i * (W - 2 * pad) / (n - 1));
  const y = (v) => H - pad - (max ? v / max * (H - 2 * pad) : 0);
  const pts = values.map((v, i) => x(i).toFixed(1) + ',' + y(v).toFixed(1)).join(' ');
  return s('svg', { class: 'spark', width: W, height: H, viewBox: '0 0 ' + W + ' ' + H, role: 'img', 'aria-label': 'Last ' + n + ' hours, peak ' + fmtN(max) },
    s('polyline', { points: pts, fill: 'none', stroke: 'var(--muted)', 'stroke-width': 2, 'stroke-linejoin': 'round', 'stroke-linecap': 'round' }),
    n ? s('circle', { cx: x(n - 1), cy: y(values[n - 1]), r: 4, fill: 'var(--accent)', stroke: 'var(--surface)', 'stroke-width': 2 }) : null);
}
function delta(now, prev, goodWhen) {
  if (!isNum(now) || !isNum(prev)) return h('span', { class: 'delta' }, 'no previous window');
  if (prev === 0) return h('span', { class: 'delta' }, now === 0 ? 'same as previous window' : 'previous window was empty');
  const pct = (now - prev) / prev * 100;
  const cls = Math.abs(pct) < 0.5 || !goodWhen ? '' : (pct > 0) === (goodWhen === 'up') ? 'good' : 'bad';
  return h('span', { class: 'delta ' + cls, 'data-delta': '' }, (pct >= 0 ? '▲ ' : '▼ ') + Math.abs(Math.round(pct)) + '% vs previous window');
}
function tile(label, value, deltaEl, extra, spark) {
  return h('div', { class: 'card tile', 'data-tile': label },
    h('div', { class: 'label' }, label),
    h('div', { class: 'row' }, h('div', null, h('div', { class: 'value' }, value), deltaEl), spark || null),
    extra || null);
}
const TOK = [['input_tokens', 'input', 'var(--s1)'], ['output_tokens', 'output', 'var(--s2)'], ['cache_read', 'cache read', 'var(--s3)'], ['cache_write', 'cache write', 'var(--s4)']];
const tokens = (o) => TOK.reduce((a, t) => a + (o[t[0]] || 0), 0);

function tilesRow(m) {
  const { now, prev, spark, window_h } = m.tiles;
  const win = window_h + ' h';
  const rate = now.requests ? now.ok / now.requests * 100 : null;
  const tot = tokens(now);
  const sk = (key) => sparkline(spark.map((p) => p[key] || 0));
  return h('div', { class: 'grid tiles' },
    tile('Requests, last ' + win, fmtN(now.requests), delta(now.requests, prev.requests),
      h('div', { class: 'kv' }, h('span', null, 'success ' + fmtPct(rate)), h('span', null, now.failed + ' failed'), now.refused ? h('span', null, now.refused + ' refused') : null),
      sk('requests')),
    tile('Tokens, last ' + win, fmtN(tot), delta(tot, tokens(prev)),
      h('div', null,
        h('div', { class: 'seg', role: 'img', 'aria-label': 'Token split' }, TOK.map((t) => tot && now[t[0]] ? h('span', { style: 'width:' + (now[t[0]] / tot * 100) + '%;background:' + t[2] }) : null)),
        h('div', { class: 'kv' }, TOK.map((t) => h('span', null, h('i', { class: 'sw', style: 'background:' + t[2] }), t[1] + ' ' + fmtN(now[t[0]]))))),
      sk('tokens')),
    tile('Cost, last ' + win, fmtUsd(now.cost_usd), delta(now.cost_usd, prev.cost_usd), null, sk('cost_usd')),
    tile('Median latency, last ' + win, fmtMs(now.p50_latency_ms), delta(now.p50_latency_ms, prev.p50_latency_ms, 'down'),
      h('div', { class: 'sub' }, 'The meter feed has no hourly latency, so no trend line.')));
}

function niceMax(v) {
  if (v <= 0) return 1;
  const p = Math.pow(10, Math.floor(Math.log10(v))), f = v / p;
  return (f <= 1 ? 1 : f <= 2 ? 2 : f <= 5 ? 5 : 10) * p;
}
function topRounded(x, y, w, hgt, r) {
  r = Math.min(r, w / 2, hgt);
  return 'M' + x + ',' + (y + hgt) + 'V' + (y + r) + 'Q' + x + ',' + y + ' ' + (x + r) + ',' + y + 'H' + (x + w - r) + 'Q' + (x + w) + ',' + y + ' ' + (x + w) + ',' + (y + r) + 'V' + (y + hgt) + 'Z';
}
const rowTokens = (r) => (r.input_tokens || 0) + (r.output_tokens || 0) + (r.cache_read || 0) + (r.cache_write || 0);

function trendBuckets(m, mode) {
  const end = tms(m.at);
  const hourEnd = Math.floor(end / 3600000) * 3600000;
  const buckets = [];
  if (mode === 'hour') {
    for (let i = 167; i >= 0; i--) buckets.push({ start: hourEnd - i * 3600000, label: '', by: dict() });
    const byStart = new Map(buckets.map((b) => [b.start, b]));
    for (const r of m.hourly) { const b = byStart.get(Math.floor(tms(r.t) / 3600000) * 3600000); if (b) b.by[r.model] = (b.by[r.model] || 0) + rowTokens(r); }
    buckets.forEach((b) => { const d = new Date(b.start); b.label = d.toLocaleDateString('en-US', { weekday: 'short', day: 'numeric' }) + ' ' + p2(d.getHours()) + ':00'; b.axis = d.getHours() === 0; b.short = d.toLocaleDateString('en-US', { weekday: 'short', day: 'numeric' }); });
  } else {
    const today = new Date(end); today.setHours(0, 0, 0, 0);
    for (let i = 6; i >= 0; i--) {
      const d = new Date(today); d.setDate(d.getDate() - i);
      buckets.push({ start: d.getTime(), end: new Date(d.getFullYear(), d.getMonth(), d.getDate() + 1).getTime(), label: d.toLocaleDateString('en-US', { weekday: 'short', month: 'short', day: 'numeric' }), by: dict(), axis: true });
    }
    for (const r of m.hourly) { const t = tms(r.t); const b = buckets.find((x) => t >= x.start && t < x.end); if (b) b.by[r.model] = (b.by[r.model] || 0) + rowTokens(r); }
  }
  return buckets;
}

function modelOrder(m) {
  const names = m.by_model.map((x) => x.model);
  for (const r of m.hourly) if (!names.includes(r.model)) names.push(r.model);
  return names;
}

function trendChart(m, width) {
  const buckets = trendBuckets(m, S.trend), models = modelOrder(m);
  const colorOf = (n) => slot('model', n);
  models.forEach(colorOf);
  const L = 44, B = 24, T = 8, R = 6, H = 220, W = Math.max(300, width);
  const total = (b) => Object.values(b.by).reduce((a, v) => a + v, 0);
  const max = niceMax(Math.max(...buckets.map(total), 0));
  const y = (v) => T + (H - T - B) * (1 - v / max);
  const slotW = (W - L - R) / buckets.length;
  const dense = S.trend === 'hour';
  const bw = dense ? Math.max(1, slotW - 1) : Math.min(24, slotW * 0.6);
  const svg = s('svg', { class: 'chart', viewBox: '0 0 ' + W + ' ' + H, role: 'img', 'aria-label': 'Tokens by model, ' + (dense ? 'hourly' : 'daily') + ', last 7 days' });
  for (let i = 0; i <= 4; i++) {
    const v = max * i / 4, yy = y(v);
    svg.append(s('line', { x1: L, x2: W - R, y1: yy, y2: yy, stroke: i ? 'var(--grid)' : 'var(--axis)', 'stroke-width': 1 }), s('text', { x: L - 6, y: yy + 4, 'text-anchor': 'end' }, fmtN(v)));
  }
  buckets.forEach((b, i) => {
    const x0 = L + i * slotW, bx = x0 + (slotW - bw) / 2;
    let acc = 0;
    const parts = models.filter((n) => b.by[n]);
    parts.forEach((n, k) => {
      const v = b.by[n], y1 = y(acc + v), y0 = y(acc), top = k === parts.length - 1;
      const gap = dense ? 0 : (k ? 2 : 0), hh = Math.max(0.5, y0 - y1 - gap);
      svg.append(top && !dense && hh > 4 ? s('path', { d: topRounded(bx, y1, bw, hh, 4), fill: colorOf(n) }) : s('rect', { x: bx, y: y1, width: bw, height: hh, fill: colorOf(n) }));
      acc += v;
    });
    if (b.axis) svg.append(s('text', { x: dense ? x0 : x0 + slotW / 2, y: H - 6, 'text-anchor': dense ? 'start' : 'middle' }, dense ? b.short : b.label));
    const hit = s('rect', { class: 'hit', x: x0, y: T, width: slotW, height: H - T - B, tabindex: dense ? null : 0, 'data-bucket': b.label, 'aria-label': b.label + ', ' + fmtN(total(b)) + ' tokens' });
    bindTip(hit, () => [b.label, models.filter((n) => b.by[n]).sort((a, c) => b.by[c] - b.by[a]).map((n) => ({ color: colorOf(n), label: n, value: fmtN(b.by[n]) })).concat([{ label: 'Total', value: fmtN(total(b)) }])]);
    svg.append(hit);
  });
  return svg;
}

function trendTable(m) {
  const buckets = trendBuckets(m, 'day');
  const models = modelOrder(m).filter((n) => buckets.some((b) => b.by[n]));
  return h('details', null, h('summary', null, 'Table view'),
    h('div', { class: 'table-wrap' }, h('table', { 'data-table': 'trend' },
      h('thead', null, h('tr', null, h('th', null, 'Day'), models.map((n) => h('th', { class: 'num' }, n)))),
      h('tbody', null, buckets.map((b) => h('tr', null, h('td', null, b.label), models.map((n) => h('td', { class: 'num' }, fmtN(b.by[n] || 0)))))))));
}

function trendCard(m, width) {
  return h('div', { class: 'card', 'data-card': 'trend' },
    h('header', null, h('h2', null, 'Tokens by model, last 7 days'),
      h('div', { class: 'filters', role: 'group', 'aria-label': 'Resolution', style: 'margin:0' },
        ['day', 'hour'].map((k) => h('button', { type: 'button', class: 'btn', 'data-trend': k, 'aria-pressed': S.trend === k, onclick: () => { S.trend = k; render(); } }, k === 'day' ? 'Day' : 'Hour')))),
    trendChart(m, width),
    h('div', { class: 'legend' }, modelOrder(m).map((n) => legendItem(slot('model', n), n))),
    trendTable(m));
}

function costCard(m) {
  const rows = m.by_model.slice().sort((a, b) => b.cost_usd - a.cost_usd);
  return h('div', { class: 'card', 'data-card': 'cost' },
    h('header', null, h('h2', null, 'Cost by model'), h('span', { class: 'sub' }, 'this cycle')),
    rows.length ? h('div', { class: 'mlist' }, rows.map((r) => h('div', { class: 'mrow' },
      h('div', { class: 'top2' }, h('span', { class: 'name' }, h('i', { class: 'sw', style: 'background:' + slot('model', r.model) }), r.model), h('b', null, fmtUsd(r.cost_usd))),
      h('div', { class: 'sub' }, fmtN(r.tokens) + ' tokens, ' + fmtN(r.requests) + ' requests, ' + Math.round(r.share * 100) + '% of cost'),
      h('div', { class: 'bar' }, h('span', { style: 'width:' + clamp(r.share * 100, 0, 100) + '%;background:' + slot('model', r.model) }))))) : h('div', { class: 'sub' }, 'No requests this cycle.'));
}

function statusInfo(st) {
  if (st === 'settled' || st === 'batch-call') return { cls: 'good', icon: '✓' };
  if (st === 'reserved') return { cls: '', icon: '…' };
  if (st === 'refused') return { cls: 'warn', icon: '!' };
  return { cls: 'bad', icon: '✕' };
}
function recordsCard(m) {
  const recs = m.records.slice().sort((a, b) => tms(b.t) - tms(a.t));
  const uniq = (k) => [...new Set(recs.map((r) => r[k]).filter(Boolean))].sort();
  const f = S.filters;
  const body = h('tbody');
  const note = h('div', { class: 'sub' });
  const more = expander('records', () => draw());
  const draw = () => {
    const rows = recs.filter((r) => (!f.model || r.model === f.model) && (!f.status || r.status === f.status) && (!f.label || (r.label || '').toLowerCase().includes(f.label.toLowerCase())));
    body.replaceChildren(...more.rows(rows).map((r) => {
      const si = statusInfo(r.status);
      const dur = r.t_end ? (tms(r.t_end) - tms(r.t)) / 1000 : null;
      return h('tr', null, h('td', null, localStamp(tms(r.t))), h('td', null, r.model), h('td', { class: 'wrap' }, r.label || ''),
        h('td', null, h('span', { class: 'chip ' + si.cls }, si.icon + ' ' + r.status + (r.http_status ? ' ' + r.http_status : ''))),
        h('td', { class: 'num' }, fmtN(r.input_tokens)), h('td', { class: 'num' }, fmtN(r.output_tokens)),
        h('td', { class: 'num' }, fmtN((r.cache_read || 0) + (r.cache_write || 0))), h('td', { class: 'num' }, fmtUsd(r.cost_usd)),
        h('td', { class: 'num' }, dur == null ? '–' : dur.toFixed(1) + ' s'));
    }));
    note.textContent = body.children.length + ' of ' + recs.length + ' records';
  };
  const select = (key, label) => h('select', { 'aria-label': label, 'data-filter': key, onchange: (e) => { f[key] = e.target.value; draw(); } },
    h('option', { value: '' }, 'All ' + label.toLowerCase() + 's'), uniq(key).map((v) => h('option', { value: v, selected: f[key] === v }, v)));
  const card = h('div', { class: 'card', 'data-card': 'records' },
    h('header', null, h('h2', null, 'Request records'), note, more.btn),
    h('div', { class: 'filters' }, select('model', 'Model'), select('status', 'Status'),
      h('input', { type: 'text', placeholder: 'Label contains', 'aria-label': 'Label', 'data-filter': 'label', value: f.label, oninput: (e) => { f.label = e.target.value; draw(); } })),
    h('div', { class: 'table-wrap' }, h('table', { 'data-table': 'records' },
      h('thead', null, h('tr', null, ['Time', 'Model', 'Label', 'Status'].map((c) => h('th', null, c)), ['Input', 'Output', 'Cache', 'Cost', 'Duration'].map((c) => h('th', { class: 'num' }, c)))), body)));
  draw();
  return card;
}

function heatmapCard(m) {
  const days = new Map(m.daily.map((d) => [d.date, d]));
  const end = new Date(tms(m.at)); const endUTC = Date.UTC(end.getUTCFullYear(), end.getUTCMonth(), end.getUTCDate());
  const first = endUTC - (52 * 7 + new Date(endUTC).getUTCDay()) * DAY;
  const C = 11, G = 3, LX = 30, TY = 18;
  const W = LX + 53 * (C + G), H = TY + 7 * (C + G);
  const max = Math.max(...m.daily.map((d) => d.tokens), 0);
  const level = (v) => !v ? 0 : clamp(Math.ceil(4 * Math.sqrt(v / max)), 1, 4);
  const svg = s('svg', { width: W, height: H, viewBox: '0 0 ' + W + ' ' + H, role: 'img', 'aria-label': 'Daily tokens over the last year' });
  ['Mon', 'Wed', 'Fri'].forEach((d, i) => svg.append(s('text', { x: 0, y: TY + (1 + i * 2) * (C + G) + C - 1 }, d)));
  let lastMonth = -1;
  for (let w = 0; w < 53; w++) {
    for (let d = 0; d < 7; d++) {
      const t = first + (w * 7 + d) * DAY;
      if (t > endUTC) continue;
      const dt = new Date(t), key = dt.toISOString().slice(0, 10), rec = days.get(key);
      if (d === 0 && dt.getUTCMonth() !== lastMonth) { lastMonth = dt.getUTCMonth(); svg.append(s('text', { x: LX + w * (C + G), y: 11 }, dt.toLocaleDateString('en-US', { month: 'short', timeZone: 'UTC' }))); }
      const v = rec ? rec.tokens : 0;
      const cell = s('rect', { class: 'cell', x: LX + w * (C + G), y: TY + d * (C + G), width: C, height: C, rx: 2, fill: 'var(--h' + level(v) + ')', tabindex: v ? 0 : null, 'data-date': key, 'data-level': level(v) });
      bindTip(cell, () => [key, [{ label: 'Tokens', value: fmtN(v) }, { label: 'Requests', value: fmtN(rec ? rec.requests : 0) }, { label: 'Cost', value: fmtUsd(rec ? rec.cost_usd : 0) }]]);
      svg.append(cell);
    }
  }
  return h('div', { class: 'card', 'data-card': 'heatmap' },
    h('header', null, h('h2', null, 'Daily tokens, last year'), h('span', { class: 'sub' }, m.daily.length + ' active days')),
    h('div', { class: 'heat-scroll' }, svg),
    h('div', { class: 'hm' }, 'Less', [0, 1, 2, 3, 4].map((l) => h('i', { style: 'background:var(--h' + l + ')' })), 'More'));
}

function usageView(view) {
  const m = feedData('meter');
  if (!m) { put(view, noData('meter', 'Usage needs the meter feed.')); return; }
  const w = view.clientWidth || 960;
  const wide = w >= 900;
  put(view, guard('Usage tiles', () => tilesRow(m)),
    h('div', { class: 'grid two' }, guard('Token trend', () => trendCard(m, wide ? Math.round((w - 16) * 0.6 - 34) : w - 34)), guard('Cost by model', () => costCard(m))),
    guard('Request records', () => recordsCard(m)), guard('Heatmap', () => heatmapCard(m)));
}

/* ---------- plans ---------- */
const STALE_S = 1800;
function harnessCard() {
  const hd = feedData('harness'), f = S.feeds.harness;
  const body = !hd
    ? h('div', { 'data-harness': 'unavailable' }, h('span', { class: 'chip warn' }, '! Unavailable'), ' ', h('span', { class: 'sub' }, f && f.error ? f.error : 'the harness feed has not been read yet'))
    : hd.agents.length ? h('div', { class: 'strip' }, hd.agents.map((a) => {
      const name = a.agent.replace(/_on_.*$/, '').split('_').map((x) => x.charAt(0).toUpperCase() + x.slice(1)).join(' ');
      const reg = a.regressed === 'on' ? ['bad', '▲ Regressed'] : a.regressed === 'off' ? ['good', '✓ Not regressed'] : ['warn', '! Unavailable'];
      return h('div', { class: 'agent', 'data-agent': a.agent }, h('b', null, name),
        h('span', { class: 'chip ' + reg[0] }, reg[1]),
        h('span', null, 'pass rate ' + (isNum(a.window_success_rate) ? fmtPct(a.window_success_rate) + ' (recent window)' : 'unavailable')),
        h('span', { class: 'sub' }, 'overall ' + (isNum(a.success_rate) ? fmtPct(a.success_rate) : 'unavailable')));
    })) : h('div', { class: 'sub' }, 'The tracker reports no agents.');
  return h('div', { class: 'card', 'data-card': 'harness' }, h('header', null, h('h2', null, 'Agent harness'), h('span', { class: 'sub' }, 'from the performance tracker')), body);
}

function planCards(q) {
  const plans = [...new Set(q.windows.map((w) => w.plan).concat((q.errors || []).map((e) => e.plan)))];
  const nowT = nowMs();
  return plans.map((plan) => {
    const rows = q.windows.filter((w) => w.plan === plan);
    const err = (q.errors || []).find((e) => e.plan === plan);
    return h('div', { class: 'card', 'data-plan': plan },
      h('header', null, h('h2', null, plan.charAt(0).toUpperCase() + plan.slice(1))),
      err ? h('p', { class: 'note bad' }, '✕ Could not be read: ' + err.error) : null,
      rows.map((w) => {
        const reset = tms(w.resets_at), len = w.length_s * 1000;
        const left = reset - nowT;
        const elapsed = len ? clamp(1 - left / len, 0, 1) : null;
        const weekly = w.window === 'week';
        const pace = weekly && elapsed != null && elapsed > 0.01 ? w.used_pct / 100 / elapsed : null;
        const readAt = tms(w.read_at || q.at);
        const stale = (nowT - readAt) / 1000 > STALE_S || left < 0;
        const sev = w.used_pct >= 90 ? 'crit' : w.used_pct >= 75 ? 'warn' : '';
        const label = w.window === '5h' ? '5-hour' : w.window === 'week' ? 'Weekly' : w.window;
        return h('div', { class: 'win', 'data-window': plan + '/' + w.window + '/' + w.scope },
          h('div', { class: 'what' }, label, h('small', null, w.scope)),
          h('div', { class: 'meter-cell' }, meterBar(w.used_pct, sev, weekly && elapsed != null ? elapsed * 100 : null)),
          h('div', { class: 'when' }, h('div', null, h('b', null, fmtPct(w.used_pct) + ' used')),
            h('div', { 'data-until': left < 0 ? '' : reset }, left < 0 ? 'reset passed' : 'resets in ' + fmtDur(left / 1000)),
            weekly ? (pace == null ? h('div', null, 'pace –') : h('div', { class: pace > 1 ? 'chip bad' : '', 'data-pace': pace.toFixed(2) }, (pace > 1 ? '▲ over pace ' : 'pace ') + pace.toFixed(2))) : null,
            stale ? h('div', { class: 'chip warn', 'data-stale': '' }, '! reading ' + fmtDur((nowT - readAt) / 1000) + ' old') : null));
      }),
      !rows.length && !err ? h('div', { class: 'sub' }, 'No windows reported.') : null);
  });
}

function plansView(view) {
  put(view, guard('Agent harness', harnessCard));
  const q = feedData('quota');
  if (!q) { put(view, noData('quota', 'Plans need the quota feed.')); return; }
  put(view, h('div', { class: 'grid cards-2' }, guard('Plan cards', () => planCards(q))));
}

/* ---------- routes ---------- */
function allEvents() {
  const map = new Map();
  for (const name of ['routes', 'router']) {
    const d = feedData(name);
    for (const e of d ? d.events : []) map.set(e.id, e);
  }
  const cutoff = nowMs() - WINDOWS[S.win] * 1000;
  return [...map.values()].filter((e) => tms(e.t) >= cutoff).sort((a, b) => tms(b.t) - tms(a.t));
}
function routeNames(events) {
  const hd = feedData('health'), cl = feedData('cloud');
  const names = [];
  const add = (n) => { if (n && !names.includes(n)) names.push(n); };
  if (hd) hd.hosts.forEach((x) => add(x.name));
  if (cl) (cl.enabled || Object.keys(cl.providers)).forEach(add);
  add('claude'); add('codex');
  events.forEach((e) => add(e.route));
  return names;
}
const median = (a) => { if (!a.length) return null; const b = a.slice().sort((x, y) => x - y); const i = Math.floor(b.length / 2); return b.length % 2 ? b[i] : (b[i - 1] + b[i]) / 2; };
function stats(evs) {
  const ok = evs.filter((e) => e.ok === true).length, failed = evs.filter((e) => e.ok === false).length;
  return { n: evs.length, ok, failed, rate: ok + failed ? ok / (ok + failed) * 100 : null, p50: median(evs.map((e) => e.latency_ms).filter(isNum)) };
}

function flowChart(events, width) {
  const names = routeNames(events);
  const callers = CALLERS.concat([...new Set(events.map((e) => e.caller))].filter((c) => !CALLERS.includes(c)));
  const cell = new Map();
  for (const e of events) { const k = e.caller + '\u0000' + e.route; if (!cell.has(k)) cell.set(k, []); cell.get(k).push(e); }
  const lTot = callers.map((c) => events.filter((e) => e.caller === c).length);
  const rTot = names.map((n) => events.filter((e) => e.route === n).length);
  const k = Math.min(6, 150 / Math.max(1, events.length)), MINH = 26, GAP = 14, PAD = 8;
  const narrow = width < 520;
  const LW = narrow ? 76 : 100, RW = narrow ? 100 : 150, NODE = 10;
  const W = Math.max(300, width);
  const layout = (tot) => { let y = PAD; return tot.map((t) => { const hh = Math.max(MINH, t * k); const r = { y, h: hh }; y += hh + GAP; return r; }); };
  const L = layout(lTot), R = layout(rTot);
  const H = Math.max(L[L.length - 1].y + L[L.length - 1].h, R[R.length - 1].y + R[R.length - 1].h) + PAD;
  const xl = LW, xr = W - RW - 2 * NODE;
  const svg = s('svg', { class: 'chart', viewBox: '0 0 ' + W + ' ' + H, role: 'img', 'aria-label': 'Calls from callers to routes, ' + events.length + ' in the window' });
  callers.forEach((c, i) => {
    svg.append(s('rect', { x: xl - NODE, y: L[i].y, width: NODE, height: L[i].h, rx: 2, fill: 'var(--ink2)' }),
      s('text', { class: 'ink', x: xl - NODE - 6, y: L[i].y + L[i].h / 2 - 2, 'text-anchor': 'end' }, c),
      s('text', { x: xl - NODE - 6, y: L[i].y + L[i].h / 2 + 11, 'text-anchor': 'end' }, fmtN(lTot[i]) + (lTot[i] === 1 ? ' call' : ' calls')));
  });
  names.forEach((n, j) => {
    const fails = events.filter((e) => e.route === n && e.ok === false).length;
    svg.append(s('rect', { x: xr + NODE, y: R[j].y, width: NODE, height: R[j].h, rx: 2, fill: slot('route', n) }),
      s('text', { class: 'ink', x: xr + 2 * NODE + 6, y: R[j].y + R[j].h / 2 - 2 }, n),
      s('text', { x: xr + 2 * NODE + 6, y: R[j].y + R[j].h / 2 + 11 }, fmtN(rTot[j]) + (rTot[j] === 1 ? ' call' : ' calls') + (fails ? ', ' + fails + ' failed' : '')));
  });
  const lUsed = callers.map(() => 0), rUsed = names.map(() => 0);
  const flows = s('g', null), pills = s('g', null);
  callers.forEach((c, i) => names.forEach((n, j) => {
    const list = cell.get(c + '\u0000' + n);
    if (!list) return;
    const w = Math.max(2, list.length * k);
    const y1 = L[i].y + lUsed[i] + w / 2, y2 = R[j].y + rUsed[j] + w / 2;
    lUsed[i] += w; rUsed[j] += w;
    const xe = xr + NODE, xm = (xl + xe) / 2, st = stats(list);
    const path = s('path', { d: 'M' + xl + ',' + y1 + 'C' + xm + ',' + y1 + ' ' + xm + ',' + y2 + ' ' + xe + ',' + y2, fill: 'none', stroke: slot('route', n), 'stroke-width': w, opacity: 0.5, 'data-flow': c + '>' + n, tabindex: 0 });
    bindTip(path, () => [c + ' to ' + n, [{ label: 'Calls', value: String(st.n) }, { label: 'Failed', value: String(st.failed) }, { label: 'Median latency', value: fmtMs(st.p50) }]]);
    flows.append(path);
    if (st.failed) {
      const py = (y1 + y2) / 2;
      pills.append(s('g', { 'data-fail': c + '>' + n }, s('rect', { x: xm - 20, y: py - 9, width: 40, height: 18, rx: 9, fill: 'var(--crit)' }), s('text', { x: xm, y: py + 4, 'text-anchor': 'middle', style: 'fill:var(--on-fill);font-weight:600' }, '✕ ' + st.failed)));
    }
  }));
  svg.insertBefore(flows, svg.firstChild);
  svg.append(pills);
  return svg;
}

function eventsTable(events) {
  const more = expander('events', () => card.replaceWith(eventsTable(events)));
  const rows = more.rows(events);
  const card = h('div', { class: 'card', 'data-card': 'events' },
    h('header', null, h('h2', null, 'Newest calls'), h('span', { class: 'sub' }, rows.length + ' of ' + events.length + ' in the window'), more.btn),
    rows.length ? h('div', { class: 'table-wrap' }, h('table', { 'data-table': 'events' },
      h('thead', null, h('tr', null, ['Time', 'Caller', 'Requested', 'Route', 'Model', 'Result'].map((c) => h('th', null, c)), h('th', { class: 'num' }, 'Latency'), h('th', null, 'Detail'))),
      h('tbody', null, rows.map((e) => {
        const si = e.ok === true ? { cls: 'good', icon: '✓', t: 'ok' } : e.ok === false ? { cls: 'bad', icon: '✕', t: 'failed' } : { cls: '', icon: '?', t: 'unknown' };
        return h('tr', null, h('td', null, localStamp(tms(e.t))), h('td', null, e.caller), h('td', { class: 'wrap' }, e.requested || ''), h('td', null, e.route), h('td', { class: 'wrap' }, e.model || ''),
          h('td', null, h('span', { class: 'chip ' + si.cls }, si.icon + ' ' + si.t + (e.status ? ' ' + e.status : ''))),
          h('td', { class: 'num' }, fmtMs(e.latency_ms)), h('td', { class: 'wrap' }, e.error || (isNum(e.tool_calls) ? e.tool_calls + ' tool calls' : '')));
      })))) : h('div', { class: 'sub' }, 'No calls in this window.'));
  return card;
}

function routerBanner() {
  const f = S.feeds.router;
  if (f && f.error) return h('p', { class: 'note bad', 'data-router': 'unreachable' }, '✕ Router unreachable: ' + f.error + (f.present ? '. Showing the last reading, ' + fmtDur(feedAge('router')) + ' old.' : '. It has not been reached yet.'));
  return null;
}

function routesView(view) {
  const noRoutes = !feedData('routes'), noRouter = !feedData('router');
  put(view, winSelect());
  if (noRoutes && noRouter) {
    put(view, routerBanner(), noData('routes', 'Routes need the pushed routes feed or the router.'), noData('router', ''));
    return;
  }
  const events = allEvents();
  const w = (view.clientWidth || 960) - 34;
  put(view, routerBanner(), noRoutes ? feedNote('routes', 'the delegate and Codex calls are missing') : null, noRouter ? feedNote('router', 'the voice and background calls are missing') : null,
    guard('Callers to routes', () => h('div', { class: 'card', 'data-card': 'flows' }, h('header', null, h('h2', null, 'Callers to routes'), h('span', { class: 'sub' }, 'width is calls; red is failed calls')),
      events.length ? flowChart(events, w) : h('div', { class: 'sub' }, 'No calls in this window.'),
      events.length ? h('div', { class: 'legend' }, routeNames(events).filter((n) => events.some((e) => e.route === n)).map((n) => legendItem(slot('route', n), n))) : null)),
    guard('Newest calls', () => eventsTable(events)));
}

/* ---------- providers ---------- */
function strip30(evs) {
  const last = evs.slice(0, 30).reverse();
  const C = 6, G = 2, W = 30 * (C + G) - G;
  const svg = s('svg', { width: W, height: 14, viewBox: '0 0 ' + W + ' 14', role: 'img', 'aria-label': 'Last ' + last.length + ' calls: ' + last.filter((e) => e.ok === true).length + ' ok, ' + last.filter((e) => e.ok === false).length + ' failed' });
  for (let i = 0; i < 30; i++) {
    const e = last[i - (30 - last.length)];
    svg.append(s('rect', { x: i * (C + G), y: e && e.ok === false ? 0 : 4, width: C, height: e && e.ok === false ? 14 : 10, rx: 2, fill: !e ? 'var(--grid)' : e.ok === true ? 'var(--good)' : e.ok === false ? 'var(--crit)' : 'var(--muted)', 'data-call': e ? (e.ok === true ? 'ok' : e.ok === false ? 'failed' : 'unknown') : 'none' }));
  }
  return svg;
}
function usageToday(p, host) {
  if (host) {
    const fl = (isNum(host.voice) ? host.voice : 0) + (isNum(host.background) ? host.background : 0);
    const ctx = isNum(host.slots_committed) && isNum(host.capacity) ? fmtN(host.slots_committed) + ' of ' + fmtN(host.capacity) + ' context tokens committed' : null;
    return h('span', null, fl + ' in flight' + (fl ? ' (' + (host.voice || 0) + ' voice, ' + (host.background || 0) + ' background)' : '') + (ctx ? ', ' + ctx : ''));
  }
  if (!p) return h('span', { class: 'sub' }, 'no limits read');
  const req = p.requests_today, lim = p.request_limit;
  const tpd = (p.quotas || []).map((q) => q.limits && q.limits.tpd).find(isNum);
  const cap = isNum(lim) ? lim : tpd || null;
  const used = isNum(lim) ? req : p.tokens_today;
  return h('div', null,
    h('div', null, fmtN(req) + (isNum(lim) ? ' of ' + fmtN(lim) + ' requests' : lim === 'per model' ? ' requests (limit per model)' : ' requests') + (isNum(p.tokens_today) ? ', ' + fmtN(p.tokens_today) + (tpd ? ' of ' + fmtN(tpd) : '') + ' tokens' : '')),
    cap ? h('div', { class: 'bar', style: 'max-width:160px' }, h('span', { style: 'width:' + clamp(used / cap * 100, 0, 100) + '%;background:' + (used / cap >= 0.9 ? 'var(--crit)' : used / cap >= 0.75 ? 'var(--warn)' : 'var(--accent)') })) : null);
}

function providersView(view) {
  const hd = feedData('health'), cl = feedData('cloud'), noRouter = !feedData('router');
  put(view, winSelect());
  if (!hd && !cl && noRouter) { put(view, routerBanner(), noData('router', 'Providers need the router feed, its cloud state and its health.'), noData('cloud', ''), noData('health', '')); return; }
  put(view, routerBanner(), noRouter ? feedNote('router', 'request counts and the call strips are missing') : null, !cl ? feedNote('cloud', 'daily limits and backoff are missing') : null, !hd ? feedNote('health', 'host state is missing') : null);
  put(view, guard('Providers table', () => {
  const events = allEvents().filter((e) => e.caller === 'voice' || e.caller === 'background');
  const rows = [];
  (hd ? hd.hosts : []).forEach((x) => rows.push({ name: x.name, kind: 'local', host: x }));
  (cl ? (cl.enabled || Object.keys(cl.providers)) : []).forEach((n) => rows.push({ name: n, kind: 'cloud', prov: own(cl.providers, n) }));
  events.forEach((e) => { if (e.route !== 'claude' && e.route !== 'codex' && !rows.some((r) => r.name === e.route)) rows.push({ name: e.route, kind: 'other' }); });
  const nowS = nowMs() / 1000;
  const stateOf = (r) => {
    if (r.host) return r.host.ceded ? ['warn', '! ceded to ' + r.host.ceded] : r.host.ready === true ? ['good', '✓ ready'] : r.host.ready === false || r.host.gate_up === false || r.host.swap_up === false ? ['bad', '✕ down'] : ['', 'unknown'];
    if (r.prov) { if (r.prov.credential_refused) return ['bad', '✕ credential refused']; if (isNum(r.prov.backoff_until) && r.prov.backoff_until > nowS) return ['warn', '! backoff']; return ['good', '✓ ready']; }
    return ['', 'not reported'];
  };
  return h('div', { class: 'card', 'data-card': 'providers' },
    h('header', null, h('h2', null, 'Providers'), h('span', { class: 'sub' }, hd ? 'voice host ' + (hd.voice_host || 'none') + (hd.voice_live ? ' (live)' : ' (not live)') + (isNum(hd.waiting) ? ', ' + hd.waiting + ' waiting' : '') : 'voice host unknown')),
    rows.length ? h('div', { class: 'table-wrap' }, h('table', { 'data-table': 'providers' },
      h('thead', null, h('tr', null, ['Route', 'Kind', 'State'].map((c) => h('th', null, c)), ['Requests', 'Success', 'p50'].map((c) => h('th', { class: 'num' }, c)), h('th', null, 'Last 30 calls'), h('th', null, 'Today'))),
      h('tbody', null, rows.map((r) => {
        const evs = events.filter((e) => e.route === r.name), st = stats(evs), sx = stateOf(r);
        return h('tr', { 'data-route': r.name }, h('td', null, h('i', { class: 'sw', style: 'background:' + slot('route', r.name) }), r.name), h('td', null, r.kind),
          h('td', null, h('span', { class: 'chip ' + sx[0] }, sx[1])),
          h('td', { class: 'num' }, fmtN(st.n)), h('td', { class: 'num' }, fmtPct(st.rate)), h('td', { class: 'num' }, fmtMs(st.p50)),
          h('td', null, strip30(evs)), h('td', { class: 'wrap' }, usageToday(r.prov, r.host)));
      })))) : h('div', { class: 'sub' }, 'No providers reported.'));
  }));
  if (cl) put(view, guard('Models in backoff', () => {
    const nowS = nowMs() / 1000;
    const back = [];
    for (const [name, m] of Object.entries(cl.models || {})) if (isNum(m.backoff_until) && m.backoff_until > nowS) back.push({ name, until: m.backoff_until, why: m.backoff_why || m.reason || '' });
    for (const [name, p] of Object.entries(cl.providers || {})) if (isNum(p.backoff_until) && p.backoff_until > nowS) back.push({ name: name + ' (whole provider)', until: p.backoff_until, why: p.backoff_reason || '' });
    return h('div', { class: 'card', 'data-card': 'backoff' }, h('header', null, h('h2', null, 'Models in backoff')),
      back.length ? h('div', { class: 'table-wrap' }, h('table', { 'data-table': 'backoff' }, h('thead', null, h('tr', null, h('th', null, 'Model'), h('th', null, 'Reason'), h('th', { class: 'num' }, 'Time left'))),
        h('tbody', null, back.sort((a, b) => a.until - b.until).map((b) => h('tr', null, h('td', { class: 'wrap' }, b.name), h('td', { class: 'wrap' }, b.why), h('td', { class: 'num', 'data-until': b.until * 1000 }, fmtDur(b.until - nowS))))))) :
        h('div', { class: 'sub' }, 'No model is in backoff.'));
  }));
}

/* ---------- shell ---------- */
function render() {
  const view = $('view');
  const keep = document.scrollingElement ? document.scrollingElement.scrollTop : 0;
  view.replaceChildren();
  document.querySelectorAll('.tabs button').forEach((b) => b.setAttribute('aria-selected', String(b.dataset.tab === S.tab)));
  guard('The header', renderHeader);
  if (S.offline) put(view, h('p', { class: 'note bad', 'data-offline': '' }, 'The add-on did not answer (' + S.offline + '). Showing the last reading.'));
  if (!S.loaded) { put(view, h('div', { class: 'nodata' }, 'Loading')); return; }
  const draw = own({ usage: usageView, plans: plansView, routes: routesView, providers: providersView }, S.tab);
  const box = h('div', { class: 'grid' });
  view.append(box);
  try { draw(box); } catch (e) { box.replaceChildren(errorCard('This view', e)); }
  if (document.scrollingElement) document.scrollingElement.scrollTop = keep;
}

function tick() {
  document.querySelectorAll('[data-age]').forEach((el) => { const a = feedAge(el.dataset.age); el.textContent = a == null ? 'no data' : fmtDur(a); });
  document.querySelectorAll('[data-until]').forEach((el) => {
    if (!el.dataset.until) return;
    const left = (Number(el.dataset.until) - nowMs()) / 1000;
    el.textContent = (el.classList.contains('num') ? '' : 'resets in ') + (left <= 0 ? 'now' : fmtDur(left));
  });
}

function apply(text) {
  let data;
  try { data = JSON.parse(text); } catch (e) { data = null; }
  if (!data || typeof data.feeds !== 'object' || data.feeds === null) { S.offline = 'the answer was not the feeds'; S.loaded = true; render(); return; }
  const now = tms(data.now);
  S.skew = isNum(now) ? now - Date.now() : 0;
  S.fetchedAt = nowMs();
  S.offline = null;
  S.feeds = data.feeds;
  S.loaded = true;
  const stable = text.replace(/"now":"[^"]*"/, '');
  if (stable !== S.raw || !S.rendered) { render(); S.rendered = true; S.raw = stable; } else guard('The header', renderHeader);
}

/* One request at a time, each with a deadline; the next poll waits for this one to end. */
let inflight = false;
async function load() {
  if (inflight || (document.hidden && S.loaded)) return;
  inflight = true;
  const abort = new AbortController();
  const timer = setTimeout(() => abort.abort(), FETCH_MS);
  try {
    const r = await fetch('api/feeds', { cache: 'no-store', signal: abort.signal });
    if (!r.ok) throw new Error('HTTP ' + r.status);
    apply(await r.text());
  } catch (e) {
    S.offline = String((e && e.message) || e);
    S.loaded = true;
    render();
  } finally {
    clearTimeout(timer);
    inflight = false;
  }
}

function setTheme(t) {
  document.documentElement.dataset.theme = t;
  $('theme').textContent = t === 'dark' ? 'Light mode' : 'Dark mode';
  store('theme', t);
}

function boot() {
  setTheme(store('theme') === 'light' ? 'light' : 'dark');
  $('theme').addEventListener('click', () => setTheme(document.documentElement.dataset.theme === 'dark' ? 'light' : 'dark'));
  const tabs = ['usage', 'plans', 'routes', 'providers'];
  const fromHash = location.hash.slice(1);
  S.tab = tabs.includes(fromHash) ? fromHash : tabs.includes(store('tab')) ? store('tab') : 'usage';
  document.querySelectorAll('.tabs button').forEach((b) => b.addEventListener('click', () => { S.tab = b.dataset.tab; store('tab', S.tab); render(); }));
  addEventListener('hashchange', () => { if (tabs.includes(location.hash.slice(1))) { S.tab = location.hash.slice(1); render(); } });
  let t;
  addEventListener('resize', () => { clearTimeout(t); t = setTimeout(render, 150); });
  document.addEventListener('visibilitychange', () => { if (!document.hidden) load(); });
  render();
  load();
  setInterval(load, POLL_MS);
  setInterval(tick, 1000);
}

window.__aiConsole = { S, render, apply, tick };
boot();
})();
