# Identity client compatibility

Every identity has two independent controls under **Client compatibility**:

```json
{"node_filter":{"auto":true,"exclude_types":["anytls","mieru"]}}
```

`auto` defaults to true, including subscriptions published before this feature.
It inspects the subscription request's User-Agent and removes nodes with a
confirmed unsupported protocol or required feature. Auto selects a full
configuration renderer; it does not silently fall back to node links when rules,
DNS or groups cannot be converted. Node filtering and configuration compatibility
are independent decisions.

Explicit node-link exports have separate encoder limits. URI validation ignores
null placeholders emitted by generic converters. Non-null
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

The authoritative node matrix is
[`src/compatibility/registry.json`](../src/compatibility/registry.json). The
independent full-configuration contract is
[`src/compatibility/config-registry.json`](../src/compatibility/config-registry.json).
The authenticated UI exposes every client, recorded version, protocol, feature
decision and evidence link. `GET /api/client-compatibility` exposes both, with
configuration contracts under `configuration`.
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
Xray, Loon or Quantumult X syntax. Shadowrocket full YAML uses the configuration
compiler; the explicit node-link exporter retains its own protocol/option limits.
Required node combinations are
checked independently: for example, `vless.reality.ws` must not inherit support
from `vless.reality.tcp`. Transport implementations without a recognized evidence
shape remain unknown, never negative merely because a source string is absent.

## Auto output

The default **Auto** option uses the existing `/sub/:token` URL (`/auto` is an
explicit alias). Selection is based on the client family, independently of whether
that exact version has a verified capability profile:

| Request | Selected output |
| --- | --- |
| Clash/Mihomo, Stash and other recognized Clash-format apps | Complete Clash YAML, retaining explicitly configured runtime settings |
| Shadowrocket | Complete Clash YAML import, including groups, expanded routing resources and DNS configuration; field-level unknowns are reported |
| Missing or unrecognized User-Agent | Complete YAML, with an unknown-client warning in preview |
| Recognized native-format family without an implemented exporter | Explicit format-unavailable error; no mislabeled YAML |

**Clash / Mihomo 完整 YAML** uses `/clash`. It preserves configured runtime fields
alongside nodes, groups, rules and DNS; it neither injects router defaults nor
blanket-removes ports, TUN, controller or DNS listener fields. Identity node
filtering and evidence-based client compatibility checks still apply. Missing or
unrecognized User-Agent selects this same renderer. The public `/router` suffix
has been removed and returns 404, with no redirect or alias. Agent revision
downloads retain their independent immutable artifact contract.

Other explicit format suffixes remain available;
`/shadowrocket-nodes` is an explicitly requested node subscription. Auto never
chooses it. Explicit node-link exports reject unrepresentable fields instead of
silently dropping options. With automatic node filtering disabled, format selection
and configuration conversion still run, but nodes are not automatically removed.
All-node removal fails with a diagnostic.

Shadowrocket [announced Clash YAML import in 2.1.60](https://t.me/ShadowrocketNews/318)
and [repeated the announcement in 2.1.95](https://t.me/ShadowrocketNews/362).
This confirms the import entry point, not complete field compatibility. Versions
before 2.1.60 have an unknown input contract and cannot select this full-output
route; they are not classified as unsupported. Unknown build numbers use
only the documented family input baseline and produce a warning; they are not
mapped to the latest release. Native rules and Clash-import parser capabilities
are recorded separately. Unknown fields, including currently unverified
PROCESS-NAME and DOMAIN-REGEX import behavior, are retained with diagnostics.
Confirmed unsupported fields or a conversion that cannot preserve a required
resource fail explicitly. Import success alone does not prove equivalent routing
or DNS behavior on a proprietary client.

The current Shadowrocket compiler expands supported GEOSITE and RULE-SET inputs
in place, retaining rule order, policy targets, logical expressions and no-resolve.
The default CN snapshot has one explicit exception: matching top-level positive
`GEOSITE,cn` references may use a classical text provider at the public, versioned
`/api/rules/geosite/<source-revision>/cn.list` mirror. The source and content hash
must match; custom sources, attributes, negation, nested logic and DNS selectors
retain the existing inline conversion path. This requested output policy does not
change Shadowrocket's unknown provider capabilities into verified support.
DNS selector expansion is a separate stage. It never replaces a domain regular
expression with URL-REGEX or imports routing rules into DNS. See
[configuration compatibility](configuration-compatibility.md) for the evidence,
exact dimensions and known boundaries.

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

Remote/file **proxy providers** are downloaded later by the client, so their node
payload cannot be inspected here. Automatic filtering reports that limitation while filtering
visible nodes. Explicit type exclusions reject such configurations rather than
claiming a complete exclusion. Import the provider as a subscription source to
include its nodes in the normal composition/filter pipeline. If affected filters
use regex semantics that cannot be evaluated equivalently, delivery fails instead
of guessing whether a group is safe. Dynamic remote membership remains unverified.

**Rule providers** used by Shadowrocket full conversion are resolved on the server.
HTTP(S) text/YAML providers with classical, domain or ipcidr behavior and inline
payloads can be expanded. Each input retains its matching type; domain wildcard
semantics are not flattened into suffix matching. MRS, client-local files, custom
GeoSite DAT decoding, custom fetch headers and client-selected fetch proxies are
currently explicit conversion limits. Missing resources, unknown source formats,
cycles, unsupported control flow and size/complexity limits fail without dropping
rules. DNS selectors that require keyword or regex matching cannot be converted
to plain exact/suffix selectors and also fail explicitly.

Default GeoSite resources use a fixed MetaCubeX full **classical** data revision,
which includes keyword and regex entries. The ordinary domain-only list or MRS is
not an equivalent replacement. A custom `geox-url.geosite` is never silently
replaced by the default dataset. Provider URLs are resolved once for a published
revision/target/compiler input, then locked in a tenant-scoped encrypted resource
snapshot. Later subscription refreshes reuse that snapshot; republishing creates
a new revision. Draft previews have separate input-keyed snapshots and keep a
bounded history. Authentication and ownership checks apply on cache hits too.
Only a resource snapshot that passes the complete pure compiler can be sealed.
A failed conversion releases its database claim, so a corrected upstream source
can be retried instead of permanently locking an unusable candidate.

Resource fetching occurs outside identity publication transactions. Database
leases coalesce concurrent cold requests and expire after a failed process. The
resolver uses the existing protected HTTP client, rejects private destinations
and redirects, and bounds each resource to 8 MiB, the total to 24 MiB, and the
source count to 32. Final output is limited to 16 MiB. Resource provenance metadata
contains aggregate counts and content hashes, never resource URLs or secrets.

Full Shadowrocket conversions share one process-wide admission slot, with at
most five seconds of queueing. Resource resolution has a 50-second deadline;
the lease owner's download stage is limited to 40 seconds. Parsing, decryption
and compilation run on a blocking worker, which retains its slot even if the
request is cancelled. Successful published views are cached by revision, base
hash, requested format and detected client. After ownership validation, a warm
view reuses the complete artifact and resource metadata without reading or
decrypting the resource snapshot again. A second cache check after admission
coalesces simultaneous misses. This bounds compilation memory independently of
the 32 MiB artifact cache.

`/sub/:token` and legacy format URLs use the request's variant. Their ETags derive
from final content and usage, and responses include `Vary: User-Agent`. GET, HEAD
and conditional GET have matching metadata. `X-Camofy-Client`,
`X-Camofy-Compatibility`, `X-Camofy-Filtered` and `X-Camofy-Format` contain normalized
diagnostics. Content type and filename follow the selected format.
Logs include only the normalized product/version and aggregate counts, never raw
headers, subscription tokens, addresses, node names or credentials.
An up-to-32 MiB / 12-entry process-local LRU avoids repeatedly compiling the same
immutable revision/client view. Its key includes normalized detection and the
requested format; its sources are immutable for that revision and compiler
process. Raw headers are never cache keys.
Authentication, revocation, current revision and usage checks still run on every
request; errors are not cached. Request views and authenticated published previews
are compiled by the current exporter, including when no node was removed. A saved
error from an older exporter does not require republishing to recover.

Agent synchronization continues to download exactly the immutable artifact/hash
advertised by its manifest. Automatic subscription negotiation does not silently
alter an Agent download. Explicit exclusions are part of the published Agent
artifact. Migration `0011_config_resource_snapshots.sql` adds an encrypted resource
cache table; it does not rewrite existing identities or revisions. Deleting the
owning identity or revision removes associated snapshots through foreign keys.

## Preview, maintenance and verification

`POST /api/bundles/:id/compatibility-preview` accepts `user_agent`, optional
`node_filter`, and `format`. It authenticates identity ownership, validates input,
rate-limits requests and returns final content plus removal reasons, unknown
capabilities and graph repairs. Full configuration results additionally carry
compiler statistics, configuration capability decisions, diagnostics and safe
resource provenance. Preview does not save or publish the identity; resolving
dependencies may create its scoped encrypted cache snapshot.

Run `node scripts/research-client-capabilities.mjs` to regenerate evidence from
public upstream sources. Immutable public source files are cached in the OS
temporary directory. Cached Git repositories refresh release tags on each run;
moved tags require review rather than a forced update.
The generated registry contains no local paths or private
configuration. Review parser changes, evidence, protocol/feature differences,
unknown entries and bounded ranges before committing a new matrix version.
Closed-source documentation rules require manual review; the script does not
invent missing version histories. The separate configuration matrix is manually
reviewed against its pinned source evidence and bounded version contracts.
Runtime requests never fetch capability evidence; they may resolve explicitly
required rule data before locking its resource snapshot.

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
including historical empty groups. A further source/compiled pair uses seven
requests through a loopback DNS stub and echo server to compare actual selected
policies: first-match ordering, exact domains, suffix domains, regex comma
quantifiers, nested AND/NOT exclusions, unresolved-domain no-resolve behavior,
and literal-IP no-resolve matching. No traffic is sent through a real proxy or
external destination. These checks do not replace import testing on proprietary
client apps.
