# Agent proxy control (protocol v2)

Identities own shared manual-group selections. Devices follow them by default;
device overrides and offline local edits affect only that device. Profiles keep
defining nodes, groups and rules, not activation or runtime selections.

## Distribution and convergence

- Public subscriptions export `default-selected` and reorder explicit group members
  for compatible clients. Independent clients may restore their own saved choices;
  subscription refresh is not a remote-control channel.
- Published revisions also contain an `agent` artifact without these preference
  changes. Agents compare the artifact hash, so selecting a node does not reload
  YAML or restart Mihomo. Revision bookkeeping still tracks the published identity.
- `selection_version` and device override versions use optimistic concurrency.
  Local offline changes are durable per-group intents; reconnect rebases those
  pending intents on current device overrides using CAS. Pending edits remain
  visible until acknowledged. Clearing an override resumes identity policy.
- A binding generation fences both commands and local intent. Reassignment clears
  cloud overrides/queue/reports; the next Agent sync discards previous local intent.
- After configuration changes, startup and each ten-second health check, the Agent
  reconciles selected groups and reads back actual choices. Each pass performs at
  most eight changes. Missing groups/nodes and mismatched readback are reported,
  not counted as applied. Stopped cores stay stopped. No forced connection teardown.
- Runtime group snapshots contain names, types, members and selections, never node
  credentials. They are sent on change and at most once per five minutes otherwise.
  Cloud pages distinguish desired selection from timestamped actual observations.

## Device RPC

`POST /api/devices/:id/rpc` accepts protocol-v2 methods `proxies.list`,
`proxies.delay` (one `name` parameter), `core.status`, `core.start`, `core.stop`,
and `core.restart`. Requests need an idempotency key. The cloud persists a bounded
per-device queue (16 pending; 64 retained records) in encrypted device data.

Every job has an ID, protocol version, binding generation, expected configuration
revision, deadline, status and result. WSS is only a notification; HTTPS and the
five-minute fallback fetch authoritative state. Jobs last ten minutes, unlike the
legacy two-minute slot. New queued core actions supersede older queued actions.
Emergency stop is processed before downloading a new configuration. Per-node
measurements run outside the control loop; a stale configuration rejects a delay
request rather than measuring a different configuration silently.

Agents journal execution before side effects. Retransmission returns a stored
receipt. An interrupted execution is `unknown`, never an automatic repeated
restart. Delivery is at-least-once, not an exactly-once promise. Cloud claim checks
binding and expiry before execution; terminal results cannot regress to executing.
Legacy Agents retain existing endpoints during rolling upgrades.

`/api/resources/:id/proxies` provides authorized identity/device views;
`PUT /api/resources/:id/selections` replaces that scope's choices with an
`expected_version`. Device-specific sync reports require a device token, not a
public subscription token. Cross-tenant requests, arbitrary RPC methods, commands
and arbitrary HTTP forwarding are forbidden.

## Local emergency access

The router page contains no subscription editor. An unbound device still uses cloud
OAuth. Bound-device core and proxy controls require a local administrator session,
plus same-origin CSRF checks; private LAN addressing alone is not authorization.

The first start creates `local-admin-key` in the Agent data directory (0600 on
Unix). Read it through the device's existing administrative channel, e.g.:

```sh
cat /jffs/camofy/config/local-admin-key
```

This is a local management key, not the cloud password or device token. Unlocking
issues a memory-only, HttpOnly, SameSite=Strict session lasting twelve hours.
Restarting the Agent clears sessions but preserves the key. Use a trusted LAN or
an SSH tunnel when accessing an HTTP local console. The Mihomo controller remains
loopback-only; inherited alternative controller listeners are removed.

## Verification

Run normal Rust tests, `control_end_to_end` against an isolated PostgreSQL,
the existing cloud/Agent integration suites with `mock-core`, frontend build/lint,
and `node --test scripts/test-agent-ui.cjs`. Browser checks cover identity selection,
device overrides/readback/delay receipts, local unlock, mobile overflow and reload.
Never run mock-core on real devices. Production Agent binaries come from tagged
GitHub Actions releases; cloud images are built on the deployment workstation.

### v0.1.4 rollout verification

The release was built by GitHub Actions and its ARMv7 archive checksum verified
before installation. Cloud and Agent integration suites passed, including two
isolated Agents following a shared identity, device-only overrides, offline local
selection/reconnect and restart receipt deduplication. Browser checks covered
desktop and mobile layouts, local unlock and refreshable detail URLs.

A physical ARMv7 router was then tested with a temporary ordinary no-TUN Profile
and identity, using the real Mihomo core. Verified shared selection with readback,
unchanged YAML hash and core PID during switching, isolated overrides, persistence
across core restart, reset-to-follow, local authenticated switching and upload,
idempotent RPC start/stop/restart/list/status, and an honest failed delay receipt.
An explicit loopback proxy request successfully reached the cloud health endpoint.
Both the proxy listener and controller remained loopback-only.

The previous identity and stopped-core state were restored; temporary resources
were deleted. Routing/firewall rules matched the maintenance baseline after
excluding counters and timestamps. Existing non-Camofy services were unchanged.
This validates the bounded control flow, not fleet-scale throughput or universal
third-party client synchronization.
