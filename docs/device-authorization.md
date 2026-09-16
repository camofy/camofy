# First-time Agent binding

The Agent includes a minimal LAN binding/status page, not the legacy configuration
UI. On an unbound Agent, `/` redirects to `/bind`. The cloud defaults to
`https://camofy.app/`, hidden in a closed **更多选项** disclosure. Router
pages never accept cloud passwords or subscription URLs. An already-bound Agent shows status and local Mihomo start/stop/restart controls;
HTTP requests cannot replace its existing binding.

## Bootstrap configuration

Create `agent.json` without cloud credentials:

```json
{
  "mihomo": "/jffs/camofy/core/mihomo",
  "data_dir": "/jffs/camofy/config",
  "web_listen": "0.0.0.0:3000",
  "controller_port": 9091,
  "dns_redirect": false
}
```

Run `camofy-agent /jffs/camofy/agent.json`, then open
`http://<router-LAN-IP>:3000/`. `web_listen` defaults to `0.0.0.0:3000`;
set it to null to disable the UI on already-bound agents. While unbound, the
Agent does not start Mihomo or restore an old configuration. The configuration
file and parent directory must be writable by the Agent.

Use a private IPv4/IPv6 address or localhost to reach this first-time interface.
Public source IPs, public Host headers and arbitrary hostnames are rejected to
reduce accidental WAN exposure and DNS-rebinding risk. Do not expose the setup
port on the WAN. The local HTTP connection assumes a trusted LAN; cloud traffic
uses HTTPS (loopback HTTP is allowed only for development).

## Authorization flow

1. The browser submits the chosen cloud and device name to the local Agent, with
   same-origin and CSRF checks. The Agent obtains a device_code/user_code from
   the cloud. It retains the device_code server-side.
2. The browser navigates to cloud `/authorize?user_code=...`. Login stays entirely
   in the cloud. The user checks the device name, LAN address and authorization
   code, then explicitly chooses an owned, published identity and approves.
3. The cloud navigates back to the local `/bind/complete?state=...` route. This is
   a constrained local-navigation extension, not an authorization-code redirect.
   State must match the initiating browser's HttpOnly/SameSite session cookie.
4. The Agent polls the token endpoint at the advertised interval, respects
   `authorization_pending`, `slow_down`, denial and expiry, and receives a
   one-time device-scoped grant over its own connection.
5. The Agent atomically saves `cloud_url`, `device_token` and device metadata (not a subscription URL),
   preserving all local settings and using mode 0600 on Unix. It starts syncing
   without a service restart. The browser removes the callback query on success.

The cloud creates a device resource and a revocable device-only access token.
Cloud identity reassignment updates its authorization transactionally and notifies
the device. No token copying/rebinding is needed. Client subscription URLs remain
a separate distribution channel. Legacy Agent URL settings upgrade automatically
on startup; the existing credential and all unrelated settings are preserved.
It cannot be used as a cloud account session or access another identity. Denial
does not create a device. Successful grant exchange consumes the device code
atomically. Retrying after an exchange response is lost or a local save fails may
require a new authorization; revoke any unused device credential in the cloud.
The chosen identity's actual configuration, including TUN policy, governs runtime;
pairing introduces no special-case network policy.

Reference: [OAuth 2.0 Device Authorization Grant, RFC 8628](https://www.rfc-editor.org/rfc/rfc8628).

## Tests / deployment boundary

Automated coverage includes expiry, denial, slow polling, CSRF, foreign identity
references, parallel one-time exchange, device-token scope/revocation, constrained
callbacks, LAN Host validation, invalid session/state, non-overwrite of existing
bindings and atomic settings preservation. Browser QA uses a disposable database
and mock core, never real traffic takeover. Existing Agent rollback/offline-restore
tests remain enabled.

Build on the workstation, not the router:

```sh
cargo zigbuild --release --locked --no-default-features --features agent --bin camofy-agent --target armv7-unknown-linux-musleabihf
```

Installing this build on an existing router requires explicit permission. Preserve
the current `agent.json`, running/last-good configuration, core and supervisor;
do not invoke the one-time legacy migration installer again.

## Core control and safety profile

Each identity has a computed, locked final system Profile: `mode: rule` and a
`prepend-rules` DIRECT entry for `CAMOFY_PUBLIC_URL`'s exact hostname (or an exact
IP-CIDR for IP origins). It cannot be reordered, disabled or overwritten by a user
Profile; the Agent reapplies the same guard after any local device overrides.
Old identities acquire the guard on next publication/read. Unsafe pre-guard
historical revisions cannot be directly rolled back. Rule order follows
[Mihomo's first-match semantics](https://wiki.metacubex.one/config/rules/).

Cloud core commands are device-scoped, expire after 120 seconds, and require an
execution acknowledgement. Only one pending command is accepted. The device
persists stop intent and processed control IDs in `control.json`; sync can stage
new YAML while stopped, without starting Mihomo. Restart retries do not execute
twice when only the acknowledgement is lost. Local commands use the same serialized
runtime path, independent of cloud connectivity, protected by LAN Host/peer, Origin
and CSRF checks. They may wait for an in-progress bounded configuration operation.
The local UI never exposes core secrets or editing of upstream subscriptions.

DIRECT protects routing policy, not DNS availability, physical connectivity or
server downtime. The emergency LAN UI must remain reachable on the trusted LAN.
Cloud controls report the last device observation; an offline device is not claimed
to have executed a command. No client logs are uploaded.
