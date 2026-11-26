// Loads the composed page in jsdom with the fixtures behind a stubbed fetch and checks that each view
// renders and that the no-data and failure states say so. Run from anywhere: node tools/page-check.mjs
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { JSDOM, VirtualConsole } from 'jsdom';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const web = join(root, 'ai_console/console/src/web');
const read = (p) => readFileSync(join(root, p), 'utf8');
const fx = (n) => JSON.parse(read(`fixtures/${n}`));
const page = readFileSync(join(web, 'index.html'), 'utf8')
  .replace('/*STYLE*/', () => readFileSync(join(web, 'style.css'), 'utf8'))
  .replace('/*SCRIPT*/', () => readFileSync(join(web, 'app.js'), 'utf8'));

const NOW = '2026-11-17T08:00:30Z';
const NOW_S = Date.parse(NOW) / 1000;
const age = (doc) => Math.max(0, NOW_S - Date.parse(doc.at) / 1000);
const present = (data) => ({ present: true, age_s: age(data), error: null, data });
const missing = (error = null) => ({ present: false, age_s: null, error, data: null });

function allFeeds() {
  const stamp = { at: '2026-11-17T08:00:20Z' };
  return {
    quota: present(fx('quota.json')),
    meter: present(fx('meter.json')),
    routes: present(fx('routes.json')),
    router: present(fx('router.json')),
    cloud: present({ ...fx('cloud.json'), ...stamp }),
    health: present({ ...fx('health.json'), ...stamp }),
    harness: present({
      v: 1, ...stamp, agents: [
        { agent: 'claude_code_on_workstation', regressed: 'off', success_rate: 88.4, window_success_rate: 90.0 },
        { agent: 'codex_on_workstation', regressed: 'on', success_rate: 80.0, window_success_rate: null },
      ],
    }),
  };
}

let failures = 0;
const check = (name, ok, extra = '') => { if (!ok) failures++; console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}${ok ? '' : ' ' + extra}`); };

async function open(feeds, url = 'http://localhost/', blockStorage = false) {
  const errors = [], fetched = [];
  const vc = new VirtualConsole();
  vc.on('jsdomError', (e) => errors.push(String(e.stack || e)));
  vc.on('error', (e) => errors.push(String(e)));
  const dom = new JSDOM(page, {
    runScripts: 'dangerously', pretendToBeVisual: true, url, virtualConsole: vc,
    beforeParse(w) {
      if (blockStorage) Object.defineProperty(w, 'localStorage', { get() { throw new Error('storage blocked'); } });
      w.fetch = async (u) => { fetched.push(new URL(u, w.location.href).pathname); return { ok: true, text: async () => JSON.stringify({ now: NOW, feeds }) }; };
      w.addEventListener('unhandledrejection', (e) => errors.push('unhandled rejection: ' + e.reason));
    },
  });
  await new Promise((r) => setTimeout(r, 80));
  const doc = dom.window.document;
  const q = (sel) => [...doc.querySelectorAll(sel)];
  const tab = async (name) => { doc.querySelector(`[data-tab=${name}]`).click(); await new Promise((r) => setTimeout(r, 10)); };
  const text = () => (doc.querySelector('header.top').textContent + ' ' + doc.getElementById('view').textContent).replace(/\s+/g, ' ');
  return { dom, doc, q, tab, text, errors, fetched, close: () => dom.window.close() };
}

const badAttr = (p) => p.q('*').flatMap((e) => [...e.attributes].filter((a) => /NaN|undefined|Infinity/.test(a.value)).map((a) => `${e.tagName}@${a.name}=${a.value.slice(0, 40)}`));
const clean = (name, p) => check(`${name}: no NaN or undefined in any attribute`, badAttr(p).length === 0, badAttr(p).slice(0, 3).join(' | ')) || check(`${name}: no view or card failed to draw`, p.q('[data-error]').length === 0, p.q('[data-error]').map((e) => e.textContent).join(' | ')) || check(`${name}: no NaN, undefined or [object]`, !/NaN|undefined|null|\[object/.test(p.text()), p.text().match(/.{20}(NaN|undefined|\[object).{20}/)?.[0]);

// Scenario 1: every feed present.
{
  const p = await open(allFeeds());
  check('header shows an age for each of the 7 feeds', p.q('[data-age]').length === 7 && p.q('[data-age]').every((e) => /^\d/.test(e.textContent)), p.q('[data-age]').map((e) => e.textContent).join(','));
  check('budget strip shows spent and budget', /spent \$20\.43/.test(p.text()) && /budget \$170/.test(p.text()), p.text().slice(0, 300));
  check('dark by default', p.doc.documentElement.dataset.theme === 'dark');

  check('usage: 4 tiles', p.q('[data-tile]').length === 4);
  check('usage: requests tile has success rate and failed count', /success 91\.7%/.test(p.q('[data-tile]')[0].textContent) && /1 failed/.test(p.q('[data-tile]')[0].textContent));
  check('usage: tokens tile shows the four-way split', ['input', 'output', 'cache read', 'cache write'].every((k) => p.q('[data-tile]')[1].textContent.includes(k)));
  check('usage: three tiles carry a 24-point sparkline', p.q('[data-tile] svg.spark').length === 3);
  check('usage: trend has 7 day buckets', p.q('[data-bucket]').length === 7);
  p.doc.querySelector('[data-trend=hour]').click();
  check('usage: hour toggle gives 168 buckets', p.q('[data-bucket]').length === 168, String(p.q('[data-bucket]').length));
  check('usage: cost-by-model lists the model with its share', /claude-haiku-5-5/.test(p.q('[data-card=cost]')[0].textContent) && /62%/.test(p.q('[data-card=cost]')[0].textContent));
  check('usage: one request record', p.q('[data-table=records] tbody tr').length === 1);
  p.doc.querySelector('[data-filter=label]').value = 'zzz';
  p.doc.querySelector('[data-filter=label]').dispatchEvent(new p.dom.window.Event('input'));
  check('usage: the label filter hides a non-matching record', p.q('[data-table=records] tbody tr').length === 0);
  check('usage: heatmap has a cell per day of the year', p.q('[data-card=heatmap] rect.cell').length >= 365, String(p.q('[data-card=heatmap] rect.cell').length));
  check('usage: the active day is shaded', p.q('[data-date="2026-11-17"]')[0]?.dataset.level === '4');
  clean('usage', p);

  await p.tab('plans');
  check('plans: two plan cards, five windows', p.q('[data-plan]').length === 2 && p.q('[data-window]').length === 5);
  check('plans: weekly pace over 1.0 is a warning', p.q('[data-window="claude/week/all-models"] [data-pace]')[0]?.classList.contains('bad'), p.q('[data-window="claude/week/all-models"]')[0]?.textContent);
  check('plans: weekly pace under 1.0 is plain', p.q('[data-window="codex/week/secondary"] [data-pace]')[0]?.dataset.pace === '0.84', p.q('[data-window="codex/week/secondary"] [data-pace]')[0]?.dataset.pace);
  check('plans: a 5-hour window has no pace', p.q('[data-window="claude/5h/session"] [data-pace]').length === 0);
  check('plans: reset countdown is shown', /resets in 1h 59m/.test(p.q('[data-window="claude/5h/session"]')[0].textContent), p.q('[data-window="claude/5h/session"]')[0].textContent);
  check('plans: an old Codex reading is marked with its age', /reading 3h 00m old/.test(p.q('[data-window="codex/5h/primary"] [data-stale]')[0]?.textContent ?? ''), p.q('[data-window="codex/5h/primary"]')[0].textContent);
  check('plans: a fresh Claude reading is not marked', p.q('[data-window="claude/5h/session"] [data-stale]').length === 0);
  check('plans: harness strip shows both agents', p.q('[data-agent]').length === 2);
  check('plans: Codex regressed, Claude not', /Regressed/.test(p.q('[data-agent=codex_on_workstation]')[0].textContent) && /Not regressed/.test(p.q('[data-agent=claude_code_on_workstation]')[0].textContent));
  check('plans: an unavailable pass rate says so', /pass rate unavailable/.test(p.q('[data-agent=codex_on_workstation]')[0].textContent));
  clean('plans', p);

  await p.tab('routes');
  p.doc.querySelector('[data-win="24h"]').click();
  const flows = p.q('[data-flow]').map((e) => e.dataset.flow).sort();
  check('routes: 24 h shows the four flows', flows.join() === 'background>openrouter,delegate>claude,delegate>codex,voice>workstation', flows.join());
  check('routes: the failed flow carries a failure count', p.q('[data-fail="background>openrouter"]')[0]?.textContent.includes('1'));
  check('routes: no failure count on a clean flow', p.q('[data-fail="voice>workstation"]').length === 0);
  check('routes: events table lists the four calls', p.q('[data-table=events] tbody tr').length === 4);
  p.doc.querySelector('[data-win="5m"]').click();
  check('routes: 5 min keeps only the router calls', p.q('[data-flow]').length === 2, String(p.q('[data-flow]').length));
  check('routes: the three callers always appear', ['voice', 'background', 'delegate'].every((c) => p.text().includes(c)));
  clean('routes', p);

  await p.tab('providers');
  p.doc.querySelector('[data-win="24h"]').click();
  check('providers: a row per host and cloud provider', p.q('[data-route]').map((e) => e.dataset.route).join() === 'workstation,gpuhost,spare,openrouter,zai', p.q('[data-route]').map((e) => e.dataset.route).join());
  check('providers: host states', /ready/.test(p.q('[data-route=workstation]')[0].textContent) && /ceded to qwen3-embed/.test(p.q('[data-route=gpuhost]')[0].textContent) && /down/.test(p.q('[data-route=spare]')[0].textContent));
  check('providers: host load is calls in flight and context tokens', /1 in flight \(1 voice, 0 background\), 2,400 of 32.8K context tokens committed/.test(p.q('[data-route=workstation]')[0].textContent));
  check('providers: a 30-slot strip per row', p.q('[data-route] svg').every((s) => s.querySelectorAll('rect').length === 30));
  check('providers: failed call colors the strip', p.q('[data-route=openrouter] [data-call=failed]').length === 1);
  check('providers: usage against the daily limit', /38 of 50 requests/.test(p.q('[data-route=openrouter]')[0].textContent), p.q('[data-route=openrouter]')[0].textContent);
  check('providers: success rate shown', /0%/.test(p.q('[data-route=openrouter]')[0].textContent) && /100%/.test(p.q('[data-route=workstation]')[0].textContent));
  check('providers: one model in backoff with time left', p.q('[data-table=backoff] tbody tr').length === 1 && /24m 30s/.test(p.q('[data-table=backoff]')[0].textContent), p.q('[data-table=backoff]')[0]?.textContent + ' / ' + p.q('[data-card=backoff]')[0]?.textContent);
  clean('providers', p);

  p.doc.getElementById('theme').click();
  check('theme toggle switches to light', p.doc.documentElement.dataset.theme === 'light');
  check('no script errors', p.errors.length === 0, p.errors.join('\n'));
  p.close();
}

// Scenario 2: nothing received yet.
{
  const none = Object.fromEntries(['quota', 'meter', 'routes', 'router', 'cloud', 'health', 'harness'].map((n) => [n, missing()]));
  const p = await open(none);
  check('no data: header shows "no data" for all 7 feeds', p.q('[data-age]').every((e) => e.textContent === 'no data') && p.q('[data-age]').length === 7);
  check('no data: budget strip says so', /Cycle budget: no data yet/.test(p.text()));
  check('no data: usage', p.q('[data-nodata=meter]').length === 1 && /No data yet/.test(p.q('[data-nodata=meter]')[0].textContent) && /has not been received/.test(p.q('[data-nodata=meter]')[0].textContent));
  await p.tab('plans');
  check('no data: plans', p.q('[data-nodata=quota]').length === 1);
  check('no data: harness shows unavailable', p.q('[data-harness=unavailable]').length === 1);
  await p.tab('routes');
  check('no data: routes', p.q('[data-nodata=routes]').length === 1 && p.q('[data-nodata=router]').length === 1);
  await p.tab('providers');
  check('no data: providers', p.q('[data-nodata=router]').length === 1 && p.q('[data-nodata=cloud]').length === 1 && p.q('[data-nodata=health]').length === 1);
  clean('no data', p);
  check('no data: no script errors', p.errors.length === 0, p.errors.join('\n'));
  p.close();
}

// Scenario 3: the router is unreachable and the harness cannot be read; one pushed feed is missing.
{
  const feeds = allFeeds();
  feeds.router = { ...feeds.router, error: 'connection refused' };
  feeds.cloud = missing('connection refused');
  feeds.health = missing('connection refused');
  feeds.harness = missing('no SUPERVISOR_TOKEN');
  feeds.routes = missing();
  const p = await open(feeds);
  await p.tab('routes');
  check('failure: router shown as a state with its last reading', /Router unreachable: connection refused/.test(p.text()) && /Showing the last reading/.test(p.text()));
  check('failure: the missing pushed feed is noted and the chart still draws', p.q('[data-nodata=routes]').length === 1 && p.q('[data-flow]').length >= 1);
  await p.tab('providers');
  check('failure: providers notes the missing cloud and hosts feeds', p.q('[data-nodata=cloud]').length === 1 && p.q('[data-nodata=health]').length === 1);
  await p.tab('plans');
  check('failure: harness unavailable carries the reason', /no SUPERVISOR_TOKEN/.test(p.q('[data-harness=unavailable]')[0].textContent));
  check('failure: header marks the broken feeds', p.q('[data-feed=router].bad').length === 1 && p.q('[data-feed=cloud].bad').length === 1);
  clean('failure', p);
  check('failure: no script errors', p.errors.length === 0, p.errors.join('\n'));
  p.close();
}

// Scenario 4: hostile strings as names; a name must be text and nothing else.
{
  const bad = ['constructor', '__proto__', 'toString', '<img src=x onerror="window.__pwned=1">'];
  const feeds = allFeeds();
  const meter = structuredClone(feeds.meter.data);
  meter.by_model = bad.map((model, i) => ({ model, requests: 5, tokens: 1000 * (i + 1), cost_usd: 1, share: 0.25 }));
  meter.hourly = bad.map((model, i) => ({ t: '2026-11-17T07:00:00Z', model, requests: 1, input_tokens: 100 * (i + 1), output_tokens: 10, cache_read: 0, cache_write: 0, cost_usd: 0.1 }));
  meter.records = bad.map((model, i) => ({ id: i, t: '2026-11-17T07:41:02Z', model, label: bad[3 - i], status: bad[i], input_tokens: 1 }));
  feeds.meter = present(meter);
  const routes = { v: 1, at: '2026-11-17T08:00:00Z', events: bad.map((route, i) => ({ id: 'h' + i, t: '2026-11-17T07:59:00Z', caller: bad[3 - i], route, model: bad[i], requested: bad[i], ok: i % 2 === 0, error: bad[i] })) };
  feeds.routes = present(routes);
  feeds.router = missing();
  const cloud = { enabled: bad, providers: Object.fromEntries(bad.map((n) => [n, { requests_today: 1, request_limit: 5 }])), models: Object.fromEntries(bad.map((n) => [n, { backoff_until: NOW_S + 600, backoff_why: n }])) };
  feeds.cloud = present({ ...cloud, at: '2026-11-17T08:00:20Z' });
  feeds.health = present({ hosts: bad.map((name) => ({ name, ready: true })), at: '2026-11-17T08:00:20Z' });
  feeds.quota = present({ v: 1, at: '2026-11-17T08:00:00Z', windows: bad.map((plan) => ({ plan, window: 'week', scope: plan, used_pct: 10, resets_at: '2026-11-23T00:00:00Z', length_s: 604800 })), errors: [] });
  const p = await open(feeds);
  for (const name of ['usage', 'plans', 'routes', 'providers']) {
    await p.tab(name);
    if (name === 'routes' || name === 'providers') p.doc.querySelector('[data-win="24h"]').click();
    check(`hostile: ${name} draws every name as text`, bad.every((n) => p.text().includes(n)) || name === 'plans' && p.text().includes('constructor'), p.text().slice(0, 200));
    check(`hostile: ${name} made no element from a name`, p.q('img').length === 0 && p.dom.window.__pwned === undefined);
    clean(`hostile ${name}`, p);
  }
  check('hostile: no script errors', p.errors.length === 0, p.errors.join('\n'));
  p.close();
}

// Scenario 5: wrong shapes reach the page; a view that cannot draw says so and the others still draw.
{
  const feeds = allFeeds();
  feeds.cloud = present({ enabled: 'a', providers: { p: null }, models: { x: null }, at: '2026-11-17T08:00:20Z' });
  feeds.health = present({ hosts: [{ name: '' }, null], at: '2026-11-17T08:00:20Z' });
  feeds.quota = present({ v: 1, at: '2026-11-17T08:00:00Z', windows: [{ plan: '', window: 5, scope: null, used_pct: 'x', resets_at: 'soon', length_s: 0 }], errors: 'none' });
  const meter = structuredClone(feeds.meter.data);
  meter.records = null;
  meter.daily = [{ date: 7 }];
  feeds.meter = present(meter);
  feeds.routes = present({ v: 1, at: '2026-11-17T08:00:00Z', events: [{ id: 'x' }, null] });
  const p = await open(feeds);
  check('shapes: usage keeps the cards that can draw', p.q('[data-card=cost]').length === 1 && p.q('[data-tile]').length === 4);
  check('shapes: usage names the cards that cannot', p.q('[data-error]').length >= 1, String(p.q('[data-error]').length));
  await p.tab('plans');
  check('shapes: plans still shows the harness strip', p.q('[data-agent]').length === 2);
  await p.tab('routes');
  check('shapes: routes shows a view-level error or its own, not a blank page', p.text().length > 100);
  await p.tab('providers');
  check('shapes: providers does not blank', p.q('[data-error], [data-card]').length >= 1);
  await p.tab('usage');
  check('shapes: the header and tabs survive', p.q('[data-age]').length === 7 && p.q('.tabs button').length === 4);
  check('shapes: no script errors', p.errors.length === 0, p.errors.join('\n'));
  p.close();
}

// Scenario 6: a feed dated ahead is shown as clock skew; events a little ahead still draw; ages are never negative.
{
  const feeds = allFeeds();
  feeds.meter = { ...feeds.meter, skew_s: 240, age_s: 0 };
  feeds.routes = present({ v: 1, at: '2026-11-17T08:04:00Z', events: [{ id: 'f1', t: '2026-11-17T08:03:00Z', caller: 'delegate', route: 'claude', ok: true }] });
  feeds.routes.skew_s = 210; feeds.routes.age_s = 0;
  const p = await open(feeds);
  check('skew: the header marks the skewed feeds', p.q('[data-feed=meter].bad').length === 1 && /clock skew/.test(p.q('[data-feed=meter]')[0].textContent) && p.q('[data-feed=routes].bad').length === 1);
  check('skew: an unskewed feed is not marked', p.q('[data-feed=quota].bad').length === 0);
  check('skew: no negative age', !/-\d/.test(p.q('[data-age]').map((e) => e.textContent).join(' ')), p.q('[data-age]').map((e) => e.textContent).join(' '));
  await p.tab('routes');
  p.doc.querySelector('[data-win="5m"]').click();
  clean('skew', p);
  check('skew: no script errors', p.errors.length === 0, p.errors.join('\n'));
  p.close();
}

// Scenario 7: behind Home Assistant's ingress the page lives under a path prefix; every request must stay under it.
{
  const prefix = '/api/hassio_ingress/TOKEN123/';
  const p = await open(allFeeds(), 'http://localhost' + prefix);
  check('ingress: the feed request is relative to the page path', p.fetched.length >= 1 && p.fetched.every((u) => u === prefix + 'api/feeds'), p.fetched.join(','));
  const urls = p.q('[src], [href], link, form').flatMap((e) => [e.getAttribute('src'), e.getAttribute('href'), e.getAttribute('action')]).filter(Boolean);
  check('ingress: no element points at an absolute path', urls.every((u) => !u.startsWith('/') && !/^[a-z]+:/i.test(u)), urls.join(','));
  await p.tab('plans');
  check('ingress: the page still draws', p.q('[data-plan]').length === 2);
  check('ingress: no script errors', p.errors.length === 0, p.errors.join('\n'));
  p.close();
}

// Scenario 8: the request records and the newest calls collapse to the newest 10; the choice is kept per list.
{
  const mk = (i) => new Date(Date.parse('2026-11-17T07:00:00Z') + i * 60000).toISOString().replace('.000', '');
  const feeds = () => {
    const f = allFeeds();
    const rec = fx('meter.json').records[0];
    f.meter = present({ ...fx('meter.json'), records: Array.from({ length: 25 }, (_, i) => ({ ...rec, id: 1000 + i, t: mk(i), t_end: null, label: 'rec-' + String(i).padStart(2, '0') })) });
    f.routes = present({ v: 1, at: '2026-11-17T08:00:20Z', events: Array.from({ length: 25 }, (_, i) => ({ id: 'e' + i, t: mk(i), caller: 'delegate', route: 'claude', model: 'ev-' + String(i).padStart(2, '0'), ok: true })) });
    f.router = missing();
    return f;
  };
  const labels = (p, table, col) => p.q(`[data-table=${table}] tbody tr`).map((r) => r.children[col].textContent);
  const p = await open(feeds());
  const recs = () => labels(p, 'records', 2), evs = () => labels(p, 'events', 4);
  const ls = () => p.dom.window.localStorage;

  check('records: collapsed by default to 10 rows', recs().length === 10, String(recs().length));
  check('records: the 10 are the newest, newest first', recs()[0] === 'rec-24' && recs()[9] === 'rec-15', recs().join());
  check('records: the toggle offers all 25', /Show all 25/.test(p.q('[data-expand=records]')[0].textContent) && p.q('[data-expand=records]')[0].getAttribute('aria-expanded') === 'false');
  check('records: the note counts the rows shown', /10 of 25 records/.test(p.q('[data-card=records] .sub')[0].textContent));
  p.q('[data-expand=records]')[0].click();
  check('records: expanded shows all 25', recs().length === 25 && recs()[24] === 'rec-00', String(recs().length));
  check('records: the toggle now collapses', /Show newest 10/.test(p.q('[data-expand=records]')[0].textContent) && p.q('[data-expand=records]')[0].getAttribute('aria-expanded') === 'true');
  check('records: the choice is kept in localStorage', ls().getItem('ai-console.open.records') === '1');
  p.doc.querySelector('[data-filter=label]').value = 'rec-0';
  p.doc.querySelector('[data-filter=label]').dispatchEvent(new p.dom.window.Event('input'));
  check('records: a filter leaving 10 rows hides the toggle', p.q('[data-table=records] tbody tr').length === 10 && p.q('[data-expand=records]')[0].hidden === true);
  p.doc.querySelector('[data-filter=label]').value = '';
  p.doc.querySelector('[data-filter=label]').dispatchEvent(new p.dom.window.Event('input'));
  p.q('[data-expand=records]')[0].click();
  check('records: collapsing returns to 10', recs().length === 10 && ls().getItem('ai-console.open.records') === '0');
  clean('records', p);

  await p.tab('routes');
  p.doc.querySelector('[data-win="24h"]').click();
  check('events: collapsed by default to 10 rows, newest first', evs().length === 10 && evs()[0] === 'ev-24' && evs()[9] === 'ev-15', evs().join());
  check('events: the toggle offers all 25 and the header counts 10 of 25', /Show all 25/.test(p.q('[data-expand=events]')[0].textContent) && /10 of 25 in the window/.test(p.q('[data-card=events] .sub')[0].textContent));
  p.q('[data-expand=events]')[0].click();
  check('events: expanded shows all 25', evs().length === 25 && evs()[24] === 'ev-00' && /25 of 25/.test(p.q('[data-card=events] .sub')[0].textContent), String(evs().length));
  check('events: the choice is kept apart from the records list', ls().getItem('ai-console.open.events') === '1' && ls().getItem('ai-console.open.records') === '0');
  p.q('[data-expand=events]')[0].click();
  check('events: collapsing returns to 10', evs().length === 10);
  p.q('[data-expand=events]')[0].click();
  clean('events', p);
  check('lists: no script errors', p.errors.length === 0, p.errors.join('\n'));
  p.close();

  const q = await open(feeds());
  q.dom.window.localStorage.setItem('ai-console.open.records', '1');
  q.dom.window.localStorage.setItem('ai-console.open.events', '1');
  await q.tab('usage');
  q.dom.window.dispatchEvent(new q.dom.window.Event('resize'));
  await new Promise((r) => setTimeout(r, 250));
  check('records: a kept expanded choice draws all rows', labels(q, 'records', 2).length === 25, String(labels(q, 'records', 2).length));
  await q.tab('routes');
  q.doc.querySelector('[data-win="24h"]').click();
  check('events: a kept expanded choice draws all rows', labels(q, 'events', 4).length === 25, String(labels(q, 'events', 4).length));
  q.close();

  const b = await open(feeds(), 'http://localhost/', true);
  check('records: with storage blocked the list is collapsed and the page draws', labels(b, 'records', 2).length === 10 && b.errors.length === 0, b.errors.join('\n'));
  b.q('[data-expand=records]')[0]?.click();
  check('records: with storage blocked the toggle does not break the page', b.errors.length === 0 && b.q('[data-card=records]').length === 1, b.errors.join('\n'));
  b.close();
}

console.log(failures ? `${failures} check(s) failed` : 'all page checks passed');
process.exit(failures ? 1 : 0);
