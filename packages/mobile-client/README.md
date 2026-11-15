# @shina/mobile-client

A portable Shina protocol client core for React Native and other non-browser
runtimes. It pairs over a tailnet, holds an authenticated resumable WebSocket
session, maintains a bounded workspace projection with per-thread history, and
exposes typed reads and durable, device-scoped command receipts. Reusing a
command UUID does not repeat its accepted dispatch. This does not guarantee
exactly-once provider tool execution.

The package ships no platform code. The native app supplies every adapter
listed below; the core fails closed if one is missing or refuses.

## Required native adapters (`SessionAdapters` / `PairingAdapters`)

- `socket` — WebSocket factory `SocketFactory`. Must support an explicit
  `Authorization` request header, or carry the credential the platform's
  mechanism allows. Browser `WebSocket` cannot set `Authorization`; do not
  substitute it.
- `transport` — `PinnedTransport` for HTTP over the encrypted tailnet
  connection (identity, pair, blobs).
  Dial `address.ip:address.port` directly, never re-resolve, proxy, or follow
  redirects. Refuse to send `Origin` headers and bound response bodies.
- `resolve` — `TailnetResolver` for MagicDNS hostnames during pairing.
  Reject results that are not tailnet addresses.
- `store` — `SecureStore` (Keychain / Keystore) for the device credential.
- `uuid` — `UuidSource` for v4 UUIDs. Must produce canonical lowercase form.
- `now` — optional `Clock`; defaults to `Date.now`.
- `timeouts` — optional `{helloMs, readMs, commandMs}` overrides.

## Pairing

`PairLink.parse` accepts only `shina://pair/v1?host=…&port=…&serverId=…&expiresAt=…#code=…`
envelopes up to 512 bytes: tailnet literals or MagicDNS names, a valid port,
a canonical UUID, a millisecond expiry, and a 12-char code in the fragment.
Codes never appear in logs, errors, or serialization (`toJSON` redacts).

`redeemPairLink(link, adapters)` resolves the host through the injected
resolver, verifies `GET /v1/identity` returns the link's `serverId`, then
POSTs `{deviceId, code, expectedServerId}` to `/v1/pair`. Redirects, public,
LAN, loopback, and unsafe addresses are refused. The issued identity and
token are validated before the credential is persisted to `store`.

## Session

`ShinaClient` negotiates `hello` (protocol major/minor, projection schema,
capabilities, limits) and pins the exact expected `serverId` and `deviceId`.
Commands are correlated by UUID request ids with deadlines; a lost
acknowledgement rejects with `UncertainCommandError` carrying the original
`commandId`. Nothing is resent with a fresh id — `receipt()`,
`resolveCommand()`, and `retryCommand()` give the durable lookup and
same-id retry path.

`mutate(method, args)` sends `{commandId, args}` for known command methods.
`read(method, params)` sends raw params for read methods. Mutations while
offline or suspended reject immediately; nothing is queued. `suspend()`
closes the socket without emitting stop commands; `resume()` reuses the
stored credential and cursors and fences stale socket callbacks. Auth denial
is terminal until the app re-pairs explicitly.

Use one `ShinaClient` instance per server. Attaching another server to an existing
instance is refused. Re-pairing the same server with a different device clears
the old command retry registry because receipts belong to the original device.

## Projection and history

`ProjectionCache` applies `subscription.patch` batches atomically (the cache
is cloned first; a bad batch leaves prior state and sequence untouched),
reassembles bounded snapshot pages up to 64 MiB measured in UTF-8 bytes, and
detects sequence/epoch gaps. Per-thread history merges stream rows and pages
by source identity, revision, and sequence; newer live rows and removals win
over older in-flight pages. Unknown event kinds advance sequence without
mutating state.

## Capabilities

Optional features are gated on the exact advertised capability strings from
`hello`, never on the negotiated minor version.

- `notification-feed-v1` — `notifications({afterSequence?, limit?})` reads
  `notifications.list`. `nextCursor` is pagination state for the current
  walk only; for durable catch-up persist the last event `seq` seen.
- `mobile-push-v1` — `registerPushNotifications(platform, token)`,
  `unregisterPushNotifications()`, `pushStatus()`, `getNotification(eventId)`,
  and `resolvePushNavigation(payload)`. Without the capability every one of
  these rejects with `CapabilityError` before sending anything.
  `register`/`unregister` go through the durable command envelope; the root
  app wires their receipts.
- Push payload data is exactly `{serverId, eventId}` with both values
  canonical UUIDs. `resolvePushNavigation` rejects malformed payloads and
  any `serverId` that is not this device's paired server before the first
  request, then fetches the current routing state with `notifications.get`.
  It never approves and never pairs from push data.
- `history-items` — required for thread history views; its absence surfaces
  `updateRequired` notices.

## Tests

```
npm install
npm run typecheck   # tsc -p tsconfig.json
npm test            # node --test "test/**/*.test.ts"
```

Ordinary `npm test` runs use in-process fakes in `test/fakes.ts` and the normalized
golden session trace in `test/fixtures/session.json`. The live-server test skips
unless the Rust integration driver supplies `SHINA_LIVE_CONFIG`. Run it with
`cargo test --test server_runtime mobile_sdk` from the repository root after
installing this package's development dependencies. It uses a temporary local
server and fake provider, not a user's profile or real Expo delivery.
