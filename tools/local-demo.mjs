// Local runs only. `mock <port>` serves what the add-on pulls (the router's /routes, /cloud and /health
// and Home Assistant's /template and /states) from the fixtures with their times moved to now.
// `seed <addon-url>` pushes quota, meter and routes with synthetic history to a running add-on, for screenshots.
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import http from 'node:http';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const fx = (n) => JSON.parse(readFileSync(join(root, 'fixtures', n), 'utf8'));
const FIXTURE_NOW = Date.parse('2026-11-17T08:00:30Z');
const ISO = /^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$/;
const iso = (ms) => new Date(ms).toISOString().replace(/\.\d+Z$/, 'Z');

function shift(v, deltaMs) {
  if (typeof v === 'string' && ISO.test(v)) return iso(Date.parse(v) + deltaMs);
  if (Array.isArray(v)) return v.map((x) => shift(x, deltaMs));
  if (v && typeof v === 'object') return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, k === 'backoff_until' && typeof x === 'number' ? x + deltaMs / 1000 : shift(x, deltaMs)]));
  return v;
}

let seed = 7;
const rnd = () => (seed = (seed * 1664525 + 1013904223) % 4294967296) / 4294967296;

function synthetic(now) {
  const hourEnd = Math.floor(now / 3600000) * 3600000;
  const models = [['claude-opus-5-5', 5], ['claude-sonnet-5-5', 1.2], ['claude-haiku-5-5', 0.25]];
  const hourly = [];
  for (let i = 167; i >= 0; i--) {
    const t = hourEnd - i * 3600000, hr = new Date(t).getUTCHours();
    const busy = hr >= 13 && hr <= 24 ? 1 : 0.25;
    models.forEach(([model, price], k) => {
      if (rnd() > 0.55 * busy + 0.1) return;
      const requests = 1 + Math.floor(rnd() * 12 * busy / (k + 1) + 1);
      const input_tokens = requests * Math.floor(2000 + rnd() * 6000), output_tokens = requests * Math.floor(150 + rnd() * 700);
      const cache_read = requests * Math.floor(8000 + rnd() * 40000), cache_write = requests * Math.floor(rnd() * 3000);
      hourly.push({ t: iso(t), model, requests, input_tokens, output_tokens, cache_read, cache_write, cost_usd: +((input_tokens * price + output_tokens * price * 5 + cache_read * price * 0.1 + cache_write * price * 1.25) / 1e6).toFixed(6) });
    });
  }
  const sum = (rows, key) => rows.reduce((a, r) => a + (r[key] || 0), 0);
  const win = (from, to) => hourly.filter((r) => Date.parse(r.t) >= from && Date.parse(r.t) < to);
  const stat = (rows, fail) => ({ requests: sum(rows, 'requests'), ok: sum(rows, 'requests') - fail, failed: fail, refused: 0, input_tokens: sum(rows, 'input_tokens'), output_tokens: sum(rows, 'output_tokens'), cache_read: sum(rows, 'cache_read'), cache_write: sum(rows, 'cache_write'), cost_usd: +sum(rows, 'cost_usd').toFixed(6), p50_latency_ms: 2200 + Math.floor(rnd() * 2400) });
  const nowRows = win(hourEnd - 23 * 3600000, hourEnd + 3600000), prevRows = win(hourEnd - 47 * 3600000, hourEnd - 23 * 3600000);
  const spark = [];
  for (let i = 23; i >= 0; i--) { const rows = win(hourEnd - i * 3600000, hourEnd - i * 3600000 + 3600000); spark.push({ t: iso(hourEnd - i * 3600000), requests: sum(rows, 'requests'), tokens: sum(rows, 'input_tokens') + sum(rows, 'output_tokens') + sum(rows, 'cache_read') + sum(rows, 'cache_write'), cost_usd: sum(rows, 'cost_usd') }); }
  const byModel = models.map(([model]) => { const rows = hourly.filter((r) => r.model === model); return { model, requests: sum(rows, 'requests'), tokens: rows.reduce((a, r) => a + r.input_tokens + r.output_tokens + r.cache_read + r.cache_write, 0), cost_usd: +sum(rows, 'cost_usd').toFixed(4) }; });
  const totalCost = sum(byModel, 'cost_usd');
  byModel.forEach((m) => { m.share = +(m.cost_usd / totalCost).toFixed(3); });
  const records = [];
  const statuses = ['settled', 'settled', 'settled', 'settled', 'refused', 'upstream-error', 'settled', 'batch-call'];
  for (let i = 0; i < 60; i++) {
    const t = now - i * 1000 * 60 * (3 + Math.floor(rnd() * 25)), m = models[Math.floor(rnd() * 3)], status = statuses[Math.floor(rnd() * statuses.length)];
    records.push({ id: 900 - i, t: iso(t), t_end: iso(t + 1000 * (1 + Math.floor(rnd() * 9))), label: ['review-', 'nightly-', 'delegate-', 'probe-'][Math.floor(rnd() * 4)] + Math.floor(rnd() * 1e6).toString(16), path: '/v1/messages', model: m[0], status, http_status: status === 'upstream-error' ? 529 : status === 'refused' ? null : 200, input_tokens: Math.floor(rnd() * 9000), output_tokens: Math.floor(rnd() * 900), cache_read: Math.floor(rnd() * 30000), cache_write: Math.floor(rnd() * 2000), cost_usd: +(rnd() * 0.05).toFixed(4), note: null });
  }
  const daily = [];
  const todayUtc = Math.floor(now / 86400000) * 86400000;
  for (let d = 365; d >= 0; d--) {
    const dow = new Date(todayUtc - d * 86400000).getUTCDay();
    if (d > 150 && rnd() < 0.8) continue;
    if (rnd() < (dow === 0 || dow === 6 ? 0.45 : 0.12)) continue;
    const requests = Math.floor(5 + rnd() * rnd() * 400);
    daily.push({ date: new Date(todayUtc - d * 86400000).toISOString().slice(0, 10), requests, tokens: requests * Math.floor(20000 + rnd() * 60000), cost_usd: +(requests * (0.01 + rnd() * 0.08)).toFixed(3) });
  }
  const cycleStart = Date.UTC(new Date(now).getUTCFullYear(), new Date(now).getUTCMonth(), 1);
  return {
    v: 1, at: iso(now),
    cycle: { start: iso(cycleStart), end: iso(Date.UTC(new Date(now).getUTCFullYear(), new Date(now).getUTCMonth() + 1, 1)), budget_usd: 169.53, spent_usd: +(totalCost * 2.6).toFixed(2), reserved_usd: 1.5 },
    tiles: { window_h: 24, now: stat(nowRows, 2), prev: stat(prevRows, 1), spark }, hourly, by_model: byModel, records, daily,
  };
}

function events(now) {
  const out = [];
  const routes = [['voice', ['workstation', 'gpuhost'], 'bonsai'], ['background', ['gpuhost', 'openrouter', 'zai', 'workstation'], 'job:coding'], ['delegate', ['claude', 'codex'], 'claude-opus-5-5']];
  for (let i = 0; i < 260; i++) {
    const [caller, rs, requested] = routes[rnd() < 0.3 ? 0 : rnd() < 0.7 ? 1 : 2];
    const route = rs[Math.floor(rnd() * rs.length)], t = now - Math.floor(rnd() * rnd() * 86400000);
    const failed = route === 'openrouter' ? rnd() < 0.35 : rnd() < 0.05;
    out.push({ id: `demo-${i}`, t: iso(t), caller, requested, route, model: route === 'claude' ? 'claude-opus-5-5' : route === 'codex' ? 'gpt-6-sol' : route === 'openrouter' ? 'nvidia/nemotron-3-ultra-550b-a55b:free' : 'bonsai', ok: !failed, status: failed ? 429 : 200, latency_ms: 300 + Math.floor(rnd() * 3000), ...(failed ? { error: 'rate limited' } : {}) });
  }
  return out;
}

const mode = process.argv[2];
if (mode === 'mock') {
  const delta = Date.now() - FIXTURE_NOW;
  const routerEvents = events(Date.now()).filter((e) => e.caller !== 'delegate');
  const body = (name) => name === 'routes' ? { v: 1, at: iso(Date.now()), capacity: 2000, events: routerEvents } : shift(fx(name + '.json'), delta);
  http.createServer((req, res) => {
    const u = new URL(req.url, 'http://x');
    const send = (code, data, text) => { res.writeHead(code, { 'content-type': text ? 'text/plain' : 'application/json' }); res.end(text ?? JSON.stringify(data)); };
    if (u.pathname === '/routes') return send(200, body('routes'));
    if (u.pathname === '/cloud') return send(200, body('cloud'));
    if (u.pathname === '/health') return send(200, body('health'));
    if (u.pathname === '/ha/template') return send(200, null, JSON.stringify(fx('ha-entities.json')));
    if (u.pathname === '/ha/states') return send(200, fx('ha-states.json'));
    send(404, { error: 'not found' });
  }).listen(Number(process.argv[3]), '127.0.0.1', () => console.log('mock upstream listening on ' + process.argv[3]));
} else if (mode === 'seed') {
  const now = Date.now(), base = process.argv[3];
  const quota = shift(fx('quota.json'), now - FIXTURE_NOW);
  const feeds = { quota, meter: synthetic(now), routes: { v: 1, at: iso(now), source: 'workstation', events: events(now).filter((e) => e.caller === 'delegate').map((e) => ({ id: e.id, t: e.t, caller: 'delegate', route: e.route, model: e.model, ok: e.ok, tool_calls: 10 + Math.floor(rnd() * 80) })) } };
  for (const [name, doc] of Object.entries(feeds)) {
    const r = await fetch(`${base}/api/feed/${name}`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(doc) });
    console.log(name, r.status, r.status === 204 ? '' : await r.text());
  }
} else {
  console.error('usage: local-demo.mjs mock <port> | seed <addon-url>');
  process.exit(2);
}
