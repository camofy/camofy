# Camofy Cloud architecture and operations

The default executable is `camofy-cloud`. `camofy-agent` is built separately with
`--no-default-features --features agent`. The cloud never runs Mihomo. The agent
includes a small LAN-only binding and core-control HTTP UI, but no upstream
subscription editor, user database, or log upload. Device OAuth credentials fetch
the assigned identity through authenticated sync APIs, independently of client URLs.
Existing local-only source files are no longer build targets; they have been left
on disk because this working tree contained uncommitted router changes before the
architecture replacement. They are not a second supported deployment mode.

## Model and composition

Each registered account is one isolated tenant. Profiles, bundles,
devices, tokens and revisions are always queried with the authenticated tenant ID.
No user-supplied tenant ID is trusted. Teams and cross-account sharing are not
implemented. A self-hosted installation has its own independent accounts/data;
it does not contact a central Camofy account server.

* Source profile: a single Clash/Mihomo YAML subscription URL,
  refresh interval (300–604800 seconds), auto-refresh flag, last fetch result.
  A source may instead be bound to a WestData panel account: every refresh then
  logs in, rewrites the stored URL with the address the panel currently serves,
  opens the panel's ten-minute update switch and fetches immediately. Credentials
  are encrypted with the resource, write-only over the API, and the fetcher is
  unchanged otherwise. See [WestData 账号订阅](westdata-source.md).
* Independent profile (`type: overlay`): arbitrary partial YAML such as nodes,
  groups, rules or runtime settings. Neither profile type has global activation.
* Identity (`kind: bundle` in the API): an ordered `profiles` array of
  `{profile_id, enabled}` bindings, shared selections and a published revision.
  Any number of subscription/independent profiles can be combined, including only
  independent profiles. Disable preserves the binding and order. Profile changes
  rebuild identities; disabled bindings do not affect their output. Creation issues
  a stable `subscription_url`; revisions retain it. In identity details, More offers
  explicit primary-link reset and individually revocable historical links. Reset
  atomically invalidates only the current primary link; all of its client formats
  need re-importing. Historical links and bound devices remain valid. No automatic
  credential cleanup or rotation occurs during deployment.
* Platform proxy: administrators share a separate encrypted platform pool of HTTP,
  HTTPS or SOCKS5 endpoints. Ordinary users cannot read or manage these records.
  One global selection controls every subscription fetch; unset or failed egress
  stops refresh, never falls back to direct. Legacy tenant proxy data is retained
  but ignored and no longer exposed. Secrets are write-only; blank edits preserve them.

Registration always defaults to `user`. Only private database operations grant
`admin`; authorization checks the current database role, including existing sessions.
Platform administration does not grant access to other tenants' subscriptions.
See [roles and global egress](admin-egress-proposal.md) for migration and verification.
* Device: bundle binding, scoped credential, last application result and optional
  explicitly requested delay measurements. No process logs or traffic history.

Enabled profiles apply in order. Maps deep-merge, scalars and other lists replace.
`proxies` and `proxy-groups` upsert by name (later replaces the entire same-named
item in its original position); `rules` concatenate. All six legacy top-level
`prepend-` / `append-` operations for rules, proxies and proxy-groups run after
ordinary keys in each profile. Prepend inserts before the accumulated list, append
after it; named directive entries replace and reposition same-named entries.
Directives never leak into rendered YAML. Null ordinary lists clear the accumulated
list; null directives do nothing. Empty YAML is a no-op. An earlier MATCH still
shadows later rules: use prepend-rules for exceptions.
Duplicate names within one input list, missing group references, cycles, missing
rule policies and invalid roots are rejected. This is structural validation, not
a replacement for a target core's semantic validation. The cloud has no Mihomo;
the agent runs `mihomo -t` before applying a candidate.

Source fetching and rendering have independent error states. A bad response does
not overwrite the source cache. An invalid combination leaves the previous
published revision intact. Publication is atomic inside a PostgreSQL transaction.
Revisions contain complete rendered artifacts and shared selections. Ten versions
per bundle are retained. A rollback changes the published revision; the next
successful profile update automatically publishes again.

## Output contracts

`GET /sub/{token}` is the default, platform-neutral identity subscription: complete
merged YAML, with exactly the same content and hash as the legacy `/router` URL.
Existing identity links are normalized on read without rotating credentials;
old `/router` links remain compatible. The URL change does not alter runtime defaults.
Optional `GET /sub/{token}/{format}` exports specific formats. Responses include ETag, private/no-cache,
Content-Disposition, `profile-update-interval: 1` (hours) and revision header.
Formats: `clash`, `router`, `shadowrocket`, `shadowrocket-nodes`.

`clash` excludes router/controller settings so a desktop client's runtime settings
remain authoritative. `router` fills missing TUN/DNS defaults without overriding
explicit cloud profiles. Agent local YAML overrides cloud runtime settings; its control API is always loopback and secret
protected. Local overrides are read on application/startup; restart the agent to
apply an edited local file when the cloud revision has not changed.

Shadowrocket nodes are base64 URI lists. Supported conversions are SS (SIP002),
VMess (v2rayN JSON URI), Trojan and VLESS with basic TCP/WS/TLS options. Full output
uses Shadowrocket's Clash-compatible YAML import, **not native .conf**. Current
full compatibility subset: inline nodes/groups and DOMAIN, DOMAIN-SUFFIX,
DOMAIN-KEYWORD, IP-CIDR, IP-CIDR6, GEOIP, MATCH rules. Proxy/rule providers are not
expanded for Shadowrocket. Unsupported fields return 422 for that format with a
diagnostic; they do not silently lose nodes or stop the Clash output. These are
export capabilities, not a claim of all-protocol/all-version client compatibility.
Device-side import tests on actual Shadowrocket and Clash Verge builds are still
required before asserting end-to-end client compatibility.

Shared selections reorder a selector's explicit node list in exported subscriptions.
Agent also applies choices through the local Mihomo API. Third-party clients may
preserve their local selector choice and only refresh subscriptions on their own
schedule; a subscription URL cannot provide remote control or realtime push into
an unmodified third-party app.

## Sync and device behavior

Agent configuration accepts one `/sub/{token}` subscription URL (also accepts legacy
`/sub/{token}/router`) and derives
the cloud origin and credential. Both identity and device subscription URLs work;
device URLs additionally permit reports and delay tests.
The agent authenticates to WebSocket `/api/sync/ws` in its first frame (`{token}`).
Notifications carry only `{"type":"changed"}`. Agent fetches `/api/sync/desired`,
then downloads merged YAML from its subscription URL and checks the revision header.
It verifies SHA-256, applies local settings, validates with Mihomo, atomically
replaces configuration, reloads, applies selections and reports success/failure.
TLS plus scoped bearer credentials authenticate distribution; hashes detect content
mismatch, they are not standalone digital signatures.

Notifications reconnect with bounded exponential backoff and jitter. Every 300
seconds the agent checks authoritative desired state regardless of WebSocket state.
Server pings every 30 seconds; peers expire after 90 seconds. Credentials are
revalidated on heartbeat; revocation blocks HTTP immediately and closes the socket
within a heartbeat. Local last-good configuration starts without needing the cloud.
Agent monitors its child every ten seconds and attempts recovery; configuration
application failure attempts to restore the last running file. A failed rollback is
reported as a failure, never as successful application.

Mihomo must already be installed. Agent does not download or upgrade executables.
Linux `dns_redirect: true` opts into a dedicated CAMOFY_DNS iptables chain (TCP/UDP
53), requiring suitable privileges/TUN support. Default is false. It cleans its
PREROUTING jumps on graceful shutdown/core exit; a service supervisor and hardware
validation are required for router production rollout. Router deployments require
explicit owner authorization and migration/backup of prior configuration first.
SIGINT and SIGTERM gracefully clean up the child core and optional DNS rules.

Delay tests are requested for an individual managed device, expire after two
minutes, test up to 32 inline nodes asynchronously, and return measurements from that device's
network. The cloud does not interpret its own TCP latency as device proxy latency.

## Deployment and scaling

See root README for Docker Compose and local commands. Use PostgreSQL 16 or later.
Schema migrations run with SQLx migration locking on startup. All replicas require
the same database and `CAMOFY_ENCRYPTION_KEY`. Resource data (including subscription
URLs, proxy credentials and caches) and revision artifacts are AES-256-GCM encrypted
at rest. Passwords use Argon2; session/device/subscription tokens are stored as hashes.
Keep the encryption key with database backups; changing it without re-encryption
makes existing data unreadable. HTTPS should terminate at your reverse proxy.

Refresh work uses an indexed durable `fetch_jobs` queue, leases, fencing claim IDs
and `FOR UPDATE SKIP LOCKED`. Manual and automatic refresh share the same fetch
function, using only the global platform selection; unset/failed proxy never falls
back to direct. Source/policy/proxy generation checks prevent stale responses from
publishing; workers poll the policy during requests and cancel changes within about
one second. Publication shares an advisory lock with other workers while platform
changes take its exclusive form; network requests do not hold that lock.
API publication serializes per account, not globally. Multiple cloud replicas can
run API and worker consumers; PostgreSQL LISTEN/NOTIFY fans out changes to locally
connected tenant WebSockets. Reconnect/periodic pulls repair missed notifications.

Per replica: 30 DB connections, default 8 fetch consumers (configurable 1–64), four
concurrent password hash operations. There is no profile-count or identity-binding
product quota. Extra issued tokens are limited to 100 per account; 100 concurrent
WebSocket connections per account. Source response
limit 4 MiB, request timeout 60 seconds, DNS/connect timeout 10 seconds, no redirects.
Set final subscription URLs explicitly. Public egress blocks private, loopback,
metadata, mapped/transition and reserved network destinations. Subscription names
are resolved using fixed HTTPS DNS with ECS disabled, not the host resolver;
failures do not fall back. All returned target addresses must be public and are
pinned for SOCKS5 and HTTP(S) proxy paths; the latter rewrites the proxy
target to a validated IP while preserving origin Host/SNI. Self-host operators may
explicitly allow private egress; never enable this for untrusted public users.

100,000 accounts is a sizing goal, not a measured capacity guarantee. Account count,
active sockets, source sizes and refresh frequencies are different load dimensions.
Before public rollout, benchmark representative data/refresh/socket rates, tune
PostgreSQL and worker counts, add reverse-proxy connection/rate limits and operator
monitoring. Authentication rate limits currently use the actual TCP peer (not
untrusted forwarded headers); behind a proxy, configure IP limiting there and size
the conservative shared application limit accordingly. Self-service email
verification/password recovery, organizations, billing, distributed artifact cache
and geo-distributed delivery are outside this initial implementation.

## API

* POST `/api/auth/register`, `/login`; GET `/me`; POST `/logout` (under `/api/auth`).
  HttpOnly SameSite=Strict cookie, seven-day sessions. Mutating cookie requests need
  an Origin matching CAMOFY_PUBLIC_URL. Session Bearer auth is supported for clients
  with a session credential. Device tokens cannot access management APIs.
* GET/POST `/api/resources`; PUT/DELETE `/api/resources/{id}`. Writes contain
  `{kind, version, data}`; updates require the latest user-edit version (409 on conflict).
* POST `/api/profiles/{id}/refresh` enqueues and returns 202.
* POST `/api/profiles/westdata-services` logs in to the panel account supplied in the
  body (`username`, `password`, optional `profile_id` to reuse the stored password) and
  returns the services it can manage: `{id, name, status, next_due}`. Uses the platform
  egress, is rate limited per user, and never returns credentials or page content.
* GET `/api/bundles/{id}/preview/{format}`, `/revisions`; POST `/rollback` with revision.
* GET `/api/bundles/{id}/subscription-links` lists historical non-device links,
  excluding the primary link. Returns label, hash identifier and creation time,
  never recoverable secrets. DELETE `/api/bundles/{id}/subscription-links/{hash}`
  revokes only a historical link in this identity (not a device or primary token).
* POST `/api/bundles/{id}/subscription-links/reset` with `{version}` replaces the
  primary link transactionally and returns the updated identity. Stale/concurrent
  resets return 409. Does not create a configuration revision or change device state.
* Legacy GET/POST `/api/tokens`; DELETE `/api/tokens/{hash}` remain compatible but
  are no longer exposed as a standalone user interface. Create with bundle_id, optional
  device_id, label. Cleartext token returned once. Creating a device token rotates
  its prior credential. Device binding through OAuth creates authorization
  automatically; reassigning its identity keeps that device credential and updates
  its scope. Unbinding (deleting the device resource) cascades credential revocation,
  without stopping local Mihomo or revoking identity subscription links.
* POST `/api/devices/{id}/test` requests measurements; no arbitrary command execution.
* GET `/api/sync/desired`, `/api/sync/revisions/{id}/{format}`; POST `/api/sync/report`.
  Reports accept status/revision/message/command_id/delays, not logs.

## Verification

`cargo test --lib --bin camofy-cloud` runs pure composition, conversion and security
tests, including local SOCKS5 handshake/target pinning and redirect rejection.
PostgreSQL integration tests are explicit:

```
cargo build --no-default-features --features agent --bin camofy-agent --example mock-core
TEST_DATABASE_URL=postgres://... CAMOFY_TEST_AGENT="$PWD/target/debug/camofy-agent" CAMOFY_TEST_CORE="$PWD/target/debug/examples/mock-core" cargo test --bin camofy-cloud cloud_end_to_end -- --ignored
CAMOFY_TEST_CORE="$PWD/target/debug/examples/mock-core" cargo test --no-default-features --features agent --bin camofy-agent -- --ignored
cd web && bun install --frozen-lockfile && bun run build && bun run lint
```

Only use a disposable DB for integration tests. Tests create their own users, mock
upstream/proxy servers, and local API listener. They verify actual proxy use,
scheduled/manual fetches, tenant isolation, fenced queue claims, version conflicts,
bad-response retention, separate exports, ETags, notification, rollback, revocation,
CSRF and encryption. With CAMOFY_TEST_AGENT/CORE set, the same test also starts a real
Agent process and verifies initial application, pushed revision convergence and
device latency reporting. The fake core is local test infrastructure, not Mihomo.
CI provisions a dedicated PostgreSQL service. See [verification record](verification.md)
for checked behavior and remaining hardware/client/load validation.
