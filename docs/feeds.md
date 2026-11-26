# Feeds

The AI console add-on (`ai_console`, port 8096 and ingress) shows three views from four feeds. Three are pushed to it by a producer you run; one it pulls from an optional model router. Every feed is a JSON object with `"v": 1` and `at` (RFC 3339 UTC, when it was produced). A field the producer cannot read is absent or null, never invented; the page shows "no data yet" per feed and the age of each feed. Fixtures in `fixtures/` are the reference for each shape and are what the page and the producers test against.

No feed carries a secret, a token, a token hash, a prompt, a reply or a task description.

## Views

- Usage: tiles with sparklines and a delta against the previous window (requests with success rate and failed count, tokens split input, output, cache read, cache write, cost, median latency), token trend as stacked bars by model with a cost-by-model list beside it, request records (filterable by model, label, status; collapsed to the newest 10 until expanded), a year heatmap of daily tokens. Source: the `meter` feed.
- Plans: one card per plan (Claude, Codex) with a bar per window (5-hour, weekly, and Claude's per-family weeks): percent used, time to reset, and for weekly windows the pace (percent used divided by percent of the window elapsed; over 1.0 is drawn as a warning). Source: the `quota` feed.
- Routes: callers on the left, routes on the right (workstation Bonsai, gpuhost Bonsai, each cloud provider, Claude, Codex), flows drawn with width by count and a failure count on each, over a chosen window (5 min, 1 h, 24 h). Sources: the `routes` feed and the router pull.

## Push endpoints (the add-on)

`POST /api/feed/quota`, `POST /api/feed/meter`, `POST /api/feed/routes`, body the feed, `content-type: application/json`, at most 4 MiB. Accepted only from addresses in the option `push_sources` (default empty: no push is accepted); others get 403. A body that fails validation gets 400 naming the field and replaces nothing; so does an `at` or an event `t` more than 5 minutes ahead of the add-on's clock. Refusals for source (403), a missing length (411) and size (413) come before any body is read. The add-on keeps the latest of each in `/data/feeds/<name>.json` and, for `routes`, merges events by `id` into a 24-hour store.

`GET /api/feeds` returns all four (the router pull included) with each one's age, for the page.

## quota

```json
{"v":1,"at":"2026-11-17T08:00:00Z","source":"workstation",
 "windows":[
  {"plan":"claude","window":"5h","scope":"session","used_pct":12.0,"resets_at":"2026-11-17T10:00:00Z","length_s":18000},
  {"plan":"claude","window":"week","scope":"all-models","used_pct":18.0,"resets_at":"2026-11-23T05:59:59Z","length_s":604800},
  {"plan":"claude","window":"week","scope":"fable","used_pct":0.0,"resets_at":"2026-11-23T05:59:59Z","length_s":604800},
  {"plan":"codex","window":"5h","scope":"primary","used_pct":31.0,"resets_at":"2026-11-17T11:12:00Z","length_s":18000,"read_at":"2026-11-17T05:00:00Z"},
  {"plan":"codex","window":"week","scope":"secondary","used_pct":77.0,"resets_at":"2026-11-17T21:49:22Z","length_s":604800,"read_at":"2026-11-17T05:00:00Z"}],
 "errors":[]}
```

`read_at` is when the figure was observed when that is older than `at` (Codex readings come from the newest rollout). `errors` is a list of `{"plan","error"}` for a plan that could not be read; its windows are then absent.

## meter

Produced by whatever meters your API spend. Times are RFC 3339 UTC; money is USD as a number rounded to 6 places.

```json
{"v":1,"at":"2026-11-17T08:00:00Z",
 "cycle":{"start":"2026-11-01T00:00:00Z","end":"2026-12-01T00:00:00Z","budget_usd":169.53,"spent_usd":20.43,"reserved_usd":0.0},
 "tiles":{"window_h":24,
   "now":{"requests":12,"ok":11,"failed":1,"refused":0,"input_tokens":52000,"output_tokens":4100,"cache_read":180000,"cache_write":9000,"cost_usd":0.41,"p50_latency_ms":3400},
   "prev":{"requests":3,"ok":3,"failed":0,"refused":0,"input_tokens":9000,"output_tokens":800,"cache_read":20000,"cache_write":3000,"cost_usd":0.06,"p50_latency_ms":2900},
   "spark":[{"t":"2026-11-16T08:00:00Z","requests":0,"tokens":0,"cost_usd":0.0}]},
 "hourly":[{"t":"2026-11-17T07:00:00Z","model":"claude-haiku-5-5","requests":4,"input_tokens":20000,"output_tokens":1500,"cache_read":60000,"cache_write":3000,"cost_usd":0.012}],
 "by_model":[{"model":"claude-haiku-5-5","requests":40,"tokens":900000,"cost_usd":0.31,"share":0.62}],
 "records":[{"id":812,"t":"2026-11-17T07:41:02Z","t_end":"2026-11-17T07:41:05Z","label":"review-abc123","path":"/v1/messages","model":"claude-haiku-5-5","status":"settled","http_status":200,"input_tokens":5000,"output_tokens":300,"cache_read":12000,"cache_write":0,"cost_usd":0.0021,"note":null}],
 "daily":[{"date":"2026-11-17","requests":12,"tokens":245100,"cost_usd":0.41}]}
```

- `tiles.now` is the last `window_h` hours, `prev` the window before it; `spark` is 24 hourly points over `now`. `failed` counts rows whose status is not settled, refused, batch-call or reserved; `refused` counts refused rows. Latency is `t_end - t` over rows with both.
- `hourly` covers the last 7 days, one row per hour and model with any request. `by_model` covers the current cycle. `records` are the newest 200 rows. `daily` covers the last 366 days, days with no request omitted.
- `tokens` is input plus output plus cache read plus both cache writes.

## routes

Pushed by the producer: the calls of Claude subagents and Codex runs over the last 24 hours. Events are merged by `id`, so a push may repeat earlier events.

```json
{"v":1,"at":"2026-11-17T08:00:00Z","source":"workstation",
 "events":[
  {"id":"claude:a0a013094295beecb","t":"2026-11-17T06:43:26Z","caller":"delegate","route":"claude","model":"claude-opus-5-5","ok":true,"tool_calls":54},
  {"id":"codex:01a1b255-85ee-7dd0-a832-fb7a6b17bb34","t":"2026-11-17T05:00:00Z","caller":"delegate","route":"codex","model":"gpt-6-sol","ok":true,"latency_ms":412000}]}
```

`ok` is false for an agent or run that ended in an error, or null when that cannot be told.

## router (pulled by the add-on)

`GET <router_url>/routes?since=<unix seconds>` on the model router, inside Home Assistant's network. The router keeps the last 2000 calls in memory (lost on restart) and returns those at or after `since`.

```json
{"v":1,"at":"2026-11-17T08:00:00Z","capacity":2000,
 "events":[
  {"id":"r-1731830400-17","t":"2026-11-17T07:59:58Z","caller":"voice","requested":"bonsai","route":"workstation","model":"bonsai","ok":true,"status":200,"latency_ms":820,"input_tokens":1200,"output_tokens":40},
  {"id":"r-1731830400-18","t":"2026-11-17T07:59:59Z","caller":"background","requested":"job:coding","route":"openrouter","model":"nvidia/nemotron-3-ultra-550b-a55b:free","ok":false,"status":429,"latency_ms":300,"error":"rate limited"}]}
```

- `caller` is `voice` or `background`; `requested` is the model or `job:<labels>` the caller asked for; `route` is the local host name the router uses or the cloud provider id; `model` is what served it. `error` is a short reason, never a body.
- The add-on polls every 15 seconds while a page is open and every 60 seconds otherwise, keeping 24 hours.

## Providers and harness (added 2026-11-17, Lewis: "You can adapt other useful views to our data as well.")

Two more views and three more pulled feeds. None is pushed. All are pulled by the add-on, so they have no `v` and `at`: the add-on stamps the time it last read each one, and that is the age `/api/feeds` reports.

- Providers: one row per route, each local host and each cloud provider. Requests, success rate and median latency over the chosen window come from the `router` feed; the strip of the last 30 calls is the newest 30 `router` events of that route (green ok, red failed, gray unknown); today's usage against the daily limits comes from `cloud` (providers) and `health` (host slots); models in backoff come from `cloud`; the host state (ready, down, ceded) comes from `health`.
- Harness strip on Plans: Claude Code and Codex pass rate and the regressed state, from the Agent Harness Performance Tracker's Home Assistant entities, source `harness`.

### cloud

`GET <router_url>/cloud` on the router, polled on the same cadence as `/routes`. Fixture `fixtures/cloud.json`. Fields the page reads: `providers.<id>` with `requests_today`, `tokens_today`, `request_limit` (a number, `"per model"` or null), `backoff_until` (unix seconds or null), `backoff_reason`, `credential_refused`, and `quotas[]` (`limits` and `remaining` keyed `rpd`, `rpm`, `tpd`, ...); `models.<provider/model>` with `backoff_until`, `backoff_why`, `routable`. Read from `router.py` `CloudRouter.state()` and `cloud_limits.py` `CloudLimits.snapshot()`.

### health

`GET <router_url>/health`, same cadence. Fixture `fixtures/health.json`. Fields the page reads: `voice_live`, `voice_host`, `waiting`, `hosts[]` with `name`, `ready`, `gate_up`, `swap_up` (true, false or null when not known), `voice` and `background` (calls in flight), `committed` and `slots_committed` (context tokens committed), `capacity` (the context size, n_ctx), `slots_busy`, `ceded` (the swap-group peer holding the card, or null), and `cloud` (`providers_enabled`, `requests_today`, `models_backing_off`). Host state: ceded when `ceded` names a peer, else ready when `ready` is true, else down when `ready`, `gate_up` or `swap_up` is false, else unknown. `fixtures/live-health.json` and `fixtures/live-cloud.json` are the live router's answers (2026-11-17, models trimmed to 25) and are accepted as they are. Read from `router.py` `LocalRouter.state()`.

### harness

Read every 5 minutes through Home Assistant's Core API behind the Supervisor proxy (`http://supervisor/core/api`, `SUPERVISOR_TOKEN`, `homeassistant_api: true`). The entities are matched by integration: `POST /template` with `{{ integration_entities('agent_harness_performance_tracker') | tojson }}` gives the tracker's entity ids, `GET /states` their states. An agent is the id's text between the domain and the suffix (`claude_code_on_workstation`); `binary_sensor.<agent>_harness_regressed` is the regressed state (`on`, `off`), `sensor.<agent>_window_success_rate` and `sensor.<agent>_success_rate` the pass rates in percent. A state of `unavailable` or `unknown`, or no token, is shown as unavailable. Fixtures `fixtures/ha-entities.json` and `fixtures/ha-states.json`.
