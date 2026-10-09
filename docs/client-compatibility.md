# Identity client compatibility

Every identity has two independent controls under **Client compatibility**:

```json
{"node_filter":{"auto":true,"exclude_types":["anytls","mieru"]}}
```

`auto` defaults to true, including subscriptions published before this feature.
It inspects the subscription request's User-Agent and removes nodes with a
confirmed unsupported protocol or required feature. Auto output additionally
excludes nodes that its selected encoder cannot represent without losing options;
these have the distinct `unsupported_output` reason, not a client capability claim.
URI validation ignores null placeholders emitted by generic converters. Non-null
exceptions are protocol-specific: VLESS accepts only inert `alterId: 0` and
`cipher: auto/none` placeholders; single-port Hysteria2 accepts an inactive
`hop-interval` only when `ports` is absent or empty. This follows the pinned
[Mihomo VLESS option structure](https://github.com/MetaCubeX/mihomo/blob/v1.19.17/adapter/outbound/vless.go)
and [Hysteria2 construction](https://github.com/MetaCubeX/mihomo/blob/v1.19.17/adapter/outbound/hysteria2.go).
Active port hopping, bandwidth controls and dialer chains remain explicit encoder
limitations; they are never silently discarded to make a node export succeed.
Explicit exclusions apply to
every client and device using this identity, even when automatic filtering is off.
They never modify a shared source or another identity. Type identifiers are
extensible; aliases such as `shadowsocks`, `hy2` and `socks` are normalized.

## Matrix and evidence

The authoritative, machine-readable matrix is
[`src/compatibility/registry.json`](../src/compatibility/registry.json).
The authenticated UI exposes every client, recorded version, protocol, feature
decision and evidence link. `GET /api/client-compatibility` exposes the same data.
Each capability has three states: supported, unsupported, or unknown. A complete
protocol parser proves absence; missing entries in a partial documentation profile
remain unknown. Protocol support does not prove support for every protocol option.

| Client family | Version evidence | Automatic behavior |
| --- | --- | --- |
| Mihomo / Clash.Meta | Tagged outbound parser and adapters | Exact stable core versions |
| Clash Meta for Android | Release tag's embedded Mihomo commit | Exact official app versions, including `.Meta` suffix |
| FlClash | Release tag's embedded fork commit | Exact official app versions |
| Clash Verge Rev | Explicit bundled core in official release notes | Exact documented app versions; independently changed cores cannot be inferred |
| Original Clash | Original release source archived by the Go module proxy | Exact open-source core versions; Premium date versions stay unknown |
| v2rayNG / Xray | Explicit core release link and tagged outbound registry | Exact documented versions; Xray's Hysteria v2 is normalized to `hysteria2` |
| Stash iOS / Mac | Official protocol and release documentation | Bounded, platform-specific version ranges |
| Shadowrocket | Official App Store history | Documented recent versions; ambiguous historical boundaries stay unknown |
| sing-box | Official outbound and AnyTLS documentation | Documented version ranges; native JSON remains a separate format |
| Surge iOS / Mac | Official protocol minimum versions | Platform and version must both be known |
| Loon, Quantumult X, v2rayN, Hiddify, NekoBox / NekoRay, ClashX Meta, legacy Clash apps | Official family references; incomplete version/core mappings | Recognized family, unknown capability; explicit exclusions still work |

Important independently verified boundaries include Mihomo's Mieru in 1.19.0,
AnyTLS in 1.19.3, Mieru UDP relay in 1.19.4, and UDP transport in 1.19.17.
Clash Meta for Android 2.10.2 embeds a parser lacking both Mieru and AnyTLS.
Stash iOS 3.3.0 adds AnyTLS, 3.3.3 adds VLESS TCP Reality, and 3.6 adds Mieru.
The matrix stores source links on each profile rather than treating app names as
timeless guarantees. See [Mihomo releases](https://github.com/MetaCubeX/mihomo/releases),
[Stash protocol documentation](https://stash.wiki/proxy-protocols/proxy-types),
[Surge protocol directory](https://manual.nssurge.com/policies/overview.html), and
[sing-box AnyTLS](https://sing-box.sagernet.org/configuration/outbound/anytls/).

The supported-client inventory is extensible, not a claim that every historical
or custom build has a known capability set. Missing, oversized, malformed,
unmapped prerelease, future, versionless or custom UAs never mean “latest”. Published
app versions marked prerelease by their publisher are labelled in the matrix when
their exact bundled core is known. An explicit
Mihomo core version takes precedence over an app bundle. Otherwise the first
product and most specific token win, preventing FlClash's trailing `clash-verge`
compatibility token from becoming the detected client. Native-format clients are
identified separately: removing nodes does not convert YAML to Surge, sing-box,
Xray, Loon or Quantumult X syntax. Existing Shadowrocket exporters retain their
own validation and supported protocol/option limits. Required combinations are
checked independently: for example, `vless.reality.ws` must not inherit support
from `vless.reality.tcp`. Transport implementations without a recognized evidence
shape remain unknown, never negative merely because a source string is absent.

## Auto output

The default **Auto** option uses the existing `/sub/:token` URL (`/auto` is an
explicit alias). Selection is based on the client family, independently of whether
that exact version has a verified capability profile:

| Request | Selected output |
| --- | --- |
| Clash/Mihomo, Stash and other recognized Clash-format apps | Clash YAML, without router-local settings |
| Shadowrocket | Base64 node subscription, without proxy groups or routing rules |
| Missing or unrecognized User-Agent | Complete YAML, with an unknown-client warning in preview |
| Recognized native-format family without an implemented exporter | Explicit format-unavailable error; no mislabeled YAML |

**完整 YAML** immediately follows Auto in the selector and uses `/router`. Other
explicit format suffixes remain available. Explicit node-link exports reject
unrepresentable fields instead of silently dropping them. With automatic filtering
disabled, Auto still selects the output format but does not discard unrepresentable
nodes; unsupported output fails with a diagnostic. All-node removal also fails.
Full Shadowrocket YAML has its own validator and does not inherit URI limitations.
Ruleset conversion is not implemented by this change; unsupported full-config rules
remain explicit errors. Auto's Shadowrocket node output does not claim to deliver
the identity's routing policy.

## Transformation and delivery

The shared pure graph transformation runs after profile composition and before
serialization. Manual policy is frozen in the encrypted revision along with its
filtered base. Per-request automatic policy uses this base and the currently
deployed matrix. A matrix update is a reviewed application release; it does not
rewrite saved identities. The matrix version recorded when publishing is
provenance, while each preview report identifies the matrix actually evaluated.
Historical rollback restores the revision's frozen manual/auto policy. Editing
and simulation refer to the current draft composition, as other identity previews
do; they need not match a manually rolled-back published revision.

The transformation filters inline nodes and inline provider payloads, removes
dependent dialer nodes, repairs group membership and direct rule targets, and
preserves group names. Group membership includes name/type filters and expanded
inline provider options; empty inline providers and their references are removed.
Empty groups and interrupted relay chains reject traffic;
they never silently fall back to direct connections. An all-removed subscription
or a dangling DNS outbound selector fails with a diagnostic. Manual changes that
cannot compile are rejected atomically. Original source data remains available.

Remote/file providers are downloaded later by the client, so their payload cannot
be inspected here. Automatic filtering reports that limitation while filtering
visible nodes. Explicit type exclusions reject such configurations rather than
claiming a complete exclusion. Import the provider as a subscription source to
include its nodes in the normal composition/filter pipeline. If affected filters
use regex semantics that cannot be evaluated equivalently, delivery fails instead
of guessing whether a group is safe. Dynamic remote membership remains unverified.

`/sub/:token` and legacy format URLs use the request's variant. Their ETags derive
from final content and usage, and responses include `Vary: User-Agent`. GET, HEAD
and conditional GET have matching metadata. `X-Camofy-Client`,
`X-Camofy-Compatibility`, `X-Camofy-Filtered` and `X-Camofy-Format` contain normalized
diagnostics. Content type and filename follow the selected format.
Logs include only the normalized product/version and aggregate counts, never raw
headers, subscription tokens, addresses, node names or credentials.
An 8 MiB / 12-entry process-local LRU avoids repeatedly compiling the same immutable
revision/client view. Its key uses normalized detection, never raw headers.
Authentication, revocation, current revision and usage checks still run on every
request; errors are not cached. Request views and authenticated published previews
are compiled by the current exporter, including when no node was removed. A saved
error from an older exporter does not require republishing to recover.

Agent synchronization continues to download exactly the immutable artifact/hash
advertised by its manifest. Automatic subscription negotiation does not silently
alter an Agent download. Explicit exclusions are part of the published Agent
artifact. There is no SQL migration or release-time user-data rewrite.

## Preview, maintenance and verification

`POST /api/bundles/:id/compatibility-preview` accepts `user_agent`, optional
`node_filter`, and `format`. It authenticates identity ownership, validates input,
rate-limits requests and returns final content plus removal reasons, unknown
capabilities and graph repairs. It does not save or publish.

Run `node scripts/research-client-capabilities.mjs` to regenerate evidence from
public upstream sources. Immutable public source files are cached in the OS
temporary directory. Cached Git repositories refresh release tags on each run;
moved tags require review rather than a forced update.
The generated registry contains no local paths or private
configuration. Review parser changes, evidence, protocol/feature differences,
unknown entries and bounded ranges before committing a new matrix version.
Closed-source documentation rules require manual review; the script does not
invent missing version histories. Runtime requests never fetch upstream evidence.

Verification includes protocol and feature boundaries, ambiguous UAs, manual/auto
independence, provider payloads, chains, empty output and DNS references. The
disposable PostgreSQL HTTP suite `compatibility_http_end_to_end` covers tenant
boundaries, per-client ETags, HEAD/304, preview, atomic rejected updates, identity
isolation, immutable Agent downloads and historical policy rollback. CI runs it
alongside the existing cloud and Agent regression suites. Browser review uses
synthetic fixtures for desktop/mobile, saving, simulation, matrix navigation,
loading, errors, keyboard focus and overflow.

CI also downloads checksum-pinned Mihomo v1.19.17 and runs
`scripts/test-mihomo-compatibility.py`: five synthetic final artifacts must pass
core configuration validation and expose `REJECT` through the loopback controller,
including historical empty groups. No traffic is sent through a real proxy. This
does not replace import testing on proprietary client apps.
