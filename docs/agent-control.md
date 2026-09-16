# Identity-owned proxy selection (v0.1.5)

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

Local controls require the `local-admin-key` from the Agent data directory and
same-origin CSRF checks. On routers it is normally read with:

```sh
cat /jffs/camofy/config/local-admin-key
```

Sessions are memory-only, HttpOnly, SameSite=Strict and last twelve hours. Use a
trusted LAN or SSH tunnel for HTTP access. Mihomo's controller stays loopback-only.

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

### v0.1.5 rollout verification

The main and tag Linux CI suites and all release builds passed. A physical ARMv7
router upgraded from v0.1.4 using the published, checksum-verified release asset.
Its historical device override was discarded and its identity selection was
automatically read back without a manual snapshot request. Changing the identity
to a nested select group updated the router's actual group and leaf selection,
produced a confirmed activity event and a successful device-side latency result.
The running YAML hash and core PID stayed unchanged during this selection update;
group/member ordering also stayed unchanged.

The old-core upgrade and new-Agent stop operation both removed the TUN interface
and policy routes before proceeding. Starting the core restored the original
identity selection and running state. No force signal or global firewall flush was
used. Production desktop/mobile checks covered read-only preview, nested path,
search, sorting and URL persistence after reload. Unrelated services were not
restarted during the cloud-only image update.
