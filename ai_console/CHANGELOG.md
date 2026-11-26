# Changelog

## 0.2.0

- The request records on Usage and the newest calls on Routes each have a control that collapses the list to its newest 10 rows. Collapsed is the default; expanded shows every record or call the view holds (the old caps of 100 and 50 are gone). The choice is remembered per list in the browser.
- `push_sources` defaults to empty, which accepts no push, and `router_url` is optional and defaults to empty, which pulls no router. Set `push_sources` to the address of the machine that runs the feed commands before pushing. An installation that never set the option must now set it.
- The repository is ready to publish: MIT licence, add-on icon and logo, and neutral example names and addresses in the fixtures, tests and documentation.

## 0.1.3

- The harness strip reads Home Assistant again: the Supervisor token is taken from s6's copy of the container environment when the process environment lacks it, which it does under s6-overlay.
- The Providers view accepts the router's real host state: `voice` and `background` are calls in flight, `committed` and `slots_committed` context tokens, `capacity` the context size, `ready`, `gate_up` and `swap_up` may be null, and `ceded` names the peer holding the card. A host row shows calls in flight and context tokens committed; a host whose readiness is not known reads unknown, not down.

## 0.1.2

- The HTTP server is hyper 1.x (HTTP/1 only) on tokio. The head has a read deadline, a size limit and a header-count limit; two different `Content-Length` values, or a line feed without a carriage return in the head, are refused with 400; the source and size checks run on the parsed head before the body is polled, and a refused sender's body is never read. The body is read under a 4 MiB limit and a deadline. Connections are async with a cap, so stalled clients no longer occupy anything but a socket. A chunked push is answered 411.
- A push that cannot be saved is answered 500 naming the failure, the last good file and reading stay, and the card shows the error. A pulled feed that cannot be saved still shows, with the failure on its card.
- `options.json` that is not a JSON object refuses the start. A stored event file over the 20,000 cap or older than 24 hours comes back capped and trimmed.

## 0.1.1

- A feed or event dated more than 5 minutes ahead of the add-on's clock is refused, naming the field; a feed a little ahead is kept and the header marks it as clock skew. The router pull's `since` comes from the router's own last `at`, never from event times, so a wrong event can no longer stop the pull.
- The HTTP server is the add-on's own, on std: every read has a deadline, a push is checked against `push_sources` and its declared size (403, 411, 413) before any body is read, and a body that stalls gets 408. A slow sender can no longer hold the workers.
- Every field the page reads is validated, in the pulled feeds as well as the pushed ones; a pulled feed that fails validation is an error state and the last good reading stays. On the page each card and view draws on its own, so one that cannot draw shows its error and the rest still draw.
- At most 20,000 events are kept in each of the routes and router stores and `/api/feeds` carries the newest 5,000 of each. A write is a snapshot written outside the store lock, with fsync of the file and its directory; a stored file that cannot be read back at start is named in the log and discarded.
- Feed names such as `constructor` or `__proto__` are plain names on the page. The page makes one request at a time with a 10 second limit.
- An entry in `push_sources` that is not an IP address, or a damaged options file, refuses the start with the entry named. Poll scheduling uses the monotonic clock.

## 0.1.0

- First release: Usage, Plans, Routes and Providers views; the quota, meter and routes push endpoints with a source check, validation and a 4 MiB cap; the router pull for calls, cloud limits and host health; the agent harness strip from Home Assistant.
