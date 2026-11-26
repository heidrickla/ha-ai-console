# AI Console

The page opens from the sidebar ("AI usage") through ingress and on the LAN at port 8096.

## Options

- `push_sources`: addresses allowed to push the quota, meter and routes feeds. A push from any other address gets 403 before its body is read and writes nothing. An entry that is not an IP address refuses the start. The add-on logs the address it saw on every push, so a deploy can confirm Docker kept the sender's address (a push through ingress shows the ingress proxy's address and is refused). Default empty, which accepts no push; add the address of the machine that runs the feed commands.
- `router_url`: the base URL of an optional model router that serves `/routes`, `/cloud` and `/health` as `docs/feeds.md` describes. The add-on reads them from it. Default empty: no router is pulled and the Routes and Providers views show only what is pushed.

## Data

Everything is kept in `/data`: `feeds/<name>.json` for each feed. Pushed and pulled call events are merged by id and kept 24 hours. The latest of each other feed replaces the last. A body that fails validation gets 400 naming the field and replaces nothing; so does a feed or event dated more than 5 minutes ahead of the add-on's clock. At most 20,000 events are kept in each of the routes and router stores, written with fsync; a stored file that cannot be read back at start is logged and discarded.

## Pushing

`POST /api/feed/quota`, `/api/feed/meter` and `/api/feed/routes` with `content-type: application/json` and a `Content-Length`, at most 4 MiB (413 before the body is read). Chunked bodies get 501. The whole head must arrive within 5 seconds and the body within 30, else 408. The shapes are in the repository's `docs/feeds.md` and `fixtures/`.

## Pulls

- The router: `GET <router_url>/routes?since=<unix seconds>`, `/cloud` and `/health`, every 60 seconds, every 15 seconds while a page has read `/api/feeds` in the last 2 minutes. An unreachable router leaves the last reading in place and the page shows the error.
- Home Assistant: through the Supervisor proxy (`homeassistant_api: true`), every 5 minutes. The tracker's entities are found by integration with `integration_entities('agent_harness_performance_tracker')`; when Home Assistant cannot be read the page shows the harness strip as unavailable.

## Endpoints

`GET /` the page, `GET /api/feeds` all seven feeds with each one's age in seconds, `skew_s` when its `at` is more than a minute ahead of the add-on's clock, and last error; the event feeds carry the newest 5,000 events and `events_total`, `GET /healthz`.

The accepted residual risk: `push_sources` trusts the sender's LAN address, and there is no authentication. A host that can take an address on the list can push feeds.
