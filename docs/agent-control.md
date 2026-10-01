# Agent configuration and identity-owned proxy selection

## Application reliability (v0.1.6)

Devices report the last successfully saved/applied revision separately from the
most recently attempted candidate. A rejected candidate never advances the former.
While the core is deliberately stopped, a validated and durably saved configuration
counts as applied; the independent core state remains `stopped`.

Failures include a stage, credential-safe reason, exit code/signal when available,
and retry time. Validation drains stdout/stderr concurrently with a bounded 32 KiB
tail per stream, recognizes memory-allocation failures even when their text falls
outside the tail, and never sends raw program output or subscription secrets to
the cloud. Unknown errors retain their stage and process status without arbitrary
configuration text. The cloud also normalizes older agents' failed reports so the
failed candidate is not shown as the last successful revision. A retained core
state includes its own confirmation time.

Repeated failures use persistent exponential backoff: 30, 60, 120 seconds, up to
30 minutes. `apply-state.json` retains at most eight failed input fingerprints.
The fingerprint includes configuration content, local overlay, core/rule-file
metadata and relevant runtime settings; a revision-only change does not launch
another validator. Changed inputs and explicit start/restart can retry immediately.
The ten-second watchdog checks pending retries while controls remain available.
The last-good cache is retained and restored independently on Agent restart.

For a router whose GeoSite index construction exceeds its memory budget, copy
`examples/router-low-memory.yaml` to a persistent device-local path and set
`local_overlay` in that device's `agent.json` to that path. It fixes
`geosite-matcher: mph` for this device without editing shared identity profiles.
Validate both cold startup and live updates on the actual device: live validation
runs alongside the existing core. This is a measured device choice, not a universal
domain-count threshold or an automatic algorithm selector.

An identity is the single authority for manual proxy-group choices. Every bound
Agent follows it. Device pages are read-only previews with actual selections,
device-side latency measurements and core controls. Local router consoles only
provide binding and emergency core start/stop/restart; they have no node selector.

## Configuration, desired choice, actual state

- Profiles define nodes/groups/rules. Identity selections are stored separately,
  versioned with optimistic concurrency and synchronized without reloading YAML.
- Old cloud/device overrides and offline local selection intents are retired.
  The cloud sends an empty override map; v0.1.5 ignores and clears old local maps.
  Old mutation endpoints reject new device choices. Upgrade Agents for full behavior.
- Cloud parses group definitions before any device reports. Static group/member
  order follows the merged configuration. Runtime dynamic-provider additions are
  deduplicated and sorted. Optional name sorting is stable and saved in the URL.
- Devices still report actual group choices: desired state alone cannot prove that
  a core applied a choice. Runtime membership supplements dynamic providers, never
  replaces the cloud configuration as the authoritative inventory.
- Start/stop/restart immediately reconciles and reports state. Configuration apply
  also reconciles; the ten-second watchdog detects subsequent runtime changes.
  Reports are retried on failure and refreshed at least every five minutes.
- Selection readback is required for success. Stopped/offline devices remain pending;
  missing nodes produce explicit errors. Clearing an identity choice reapplies the
  static configuration default, not a device's old selection cache. Automatic groups
  remain automatic and may legitimately use different exits across networks.
- Public subscriptions export defaults, but cannot enforce live synchronization in
  independent clients such as Clash Verge Rev that restore their own local cache.

## UI and activity

Only one group's nodes are expanded at a time. Group navigation, node filtering,
bounded rendering, actual selection highlights and nested exit paths replace the
previous full-page stack. Selected group, activity tab and sort order survive reload.
Devices link to the identity editor instead of presenting a competing selector.

Selection events are bounded to 80 entries per scope and contain time, group,
previous/next choice, version and source. Device confirmations are recorded only
after matching version and actual readback. Events superseded before confirmation
are not reported as successful. The activity tab combines these with bounded RPC
receipts; automatic snapshot/status queries do not flood the activity list. These
are control-plane events, not proxy traffic logs.

## RPC and local access

Agent-initiated WSS notifies; HTTPS fetches authoritative state/claims work/posts
results. Five-minute fallback remains. Protocol-v2 jobs have binding-generation
fencing, IDs, idempotency keys, deadlines, expected configuration revision and
durable receipts. Duplicate restart requests are not replayed. A restart interrupted
before confirmation is unknown, not success. Delay measurements run outside the
control loop. No shell execution or arbitrary HTTP forwarding is exposed.

Local controls are available to anyone who can access the router's LAN console.
State-changing requests retain same-origin CSRF checks. Use a trusted LAN or SSH
tunnel for HTTP access. Mihomo's controller stays loopback-only.

## Graceful shutdown and upgrades

Linux Agents send SIGTERM to the child and await exit, allowing Mihomo to clean up
its own TUN routes/firewall state. They never force-kill a live core or enable
kill-on-drop for it. After a 30-second timeout, a control action fails visibly and
retains the child handle. Agent process shutdown continues waiting/retrying rather
than abandoning the child. Only Agent-owned DNS redirect rules are removed.
Mihomo handles SIGTERM through its shutdown path ([upstream source](https://github.com/MetaCubeX/mihomo/blob/Meta/main.go)).

For pre-v0.1.5 upgrades, do not ask the old Agent to stop a live TUN core: that code
used forced termination. Gracefully stop the core while preventing the old watchdog
from restarting it, verify cleanup, then replace the Agent. Preserve binding,
identity choices and the original running/stopped intent. Never flush unrelated
iptables tables or install unverified binaries. Published router binaries are built
by GitHub Actions; cloud images are built on the deployment workstation.

Verification covers migration of old overrides, first snapshot without RPC,
readback-confirmed activity, tenant isolation, stable ordering, responsive browser
interaction and a Unix child cleanup trap before termination returns. Use an isolated
database/mock core for automated tests and real hardware for rollout verification.
