# Profile Store (first release)

## User workflow

Open **Profile 商店**, inspect a package's scope, complete rule list, immutable version,
sources and license, then add it to your Profiles. Installation alone never changes an
identity or binds a device. In an identity, associate that Profile, choose a policy,
preview the final configuration, and save. Refreshing detail URLs preserves navigation.

An installed Profile is shared by its referring identities. Its enabled flag, order and
policy are identity-local. DIRECT/REJECT packages have an explicit default; routing
packages require a valid group/node name from the composed identity. Later
`prepend-rules` components precede earlier components. The system cloud DIRECT
protection remains first. Preview warns about identical/suffix-shadowed rules and
rules after MATCH; it is not a complete semantic conflict solver for all Mihomo rules.

The Profile detail page supports a reviewed version switch (including older versions),
and copying into an independent editable Profile. Upgrades validate all enabled
dependent identities and commit their new revisions in one tenant transaction. The
preview digest rejects stale changes. The policy remains manual after upgrading or
downgrading. Install again for a second independently pinned instance.

**Package version rollback** and **identity publication rollback** are different:
the former changes the managed Profile's pinned version; the latter serves the
stored immutable historical output without modifying Profiles, and normal future
Profile changes can still rebuild that identity (the history dialog explains this).
Rule files are inline in this release, so historical output has no moving remote
rule-file dependencies. Revision catalog locks record installation/version/hash and
identity-local parameters in order.

## Data model

- `catalog_packages`: stable slug, owner (private), public publisher nickname.
- `catalog_versions`: immutable version and manifest, including provenance/license.
- `catalog_artifacts`: SHA256-addressed normalized structured rule content.
- `catalog_version_artifacts`: version → artifact relations (initial role: rules).
- `catalog_publishers`: explicit operator approval, not a self-service role flag.
- Existing Profile: `origin=store`, `store={slug,version_id,update_policy:manual}`.
  Hydrated `_package` is read-only response metadata, never encrypted into the
  tenant Profile or accepted from a client as authoritative content.
- Existing identity bindings: optional `parameters={policy}`.
- Existing revisions: `catalog_lock` dependency snapshot. Package changes that
  produce equivalent rules still create a distinct provenance revision.
- Existing user: `nickname`, editable through `/account`; email cannot be edited.

Content is shared globally, not fetched separately for every tenant. Installation and
upgrades never use an airport subscription proxy. Publication is available only to
approved accounts; normal users can read/install but cannot publish arbitrary code.
Publisher email and owner identifiers are absent from public package responses.

## API

All store routes require login. Mutations use existing cookie/origin CSRF protection
or an authenticated bearer session. Existing tenant ownership and optimistic resource
versions apply.

| Route | Purpose |
| --- | --- |
| `GET /api/store/packages` | Latest published versions (curated catalog, first 100 packages) |
| `GET /api/store/packages/:slug` | Immutable versions, rules, provenance, license |
| `POST /api/store/packages` | Approved publisher creates `{slug,version,manifest,rules}` |
| `POST /api/store/install` | `{version_id,profile_id}`; caller-generated UUID makes retries idempotent |
| `POST /api/store/identity-preview` | `{data}`; unsaved identity outputs and diagnostics |
| `POST /api/profiles/:id/upgrade-preview` | `{version,version_id}`; affected identities and digest |
| `POST /api/profiles/:id/upgrade` | Same fields plus `preview_digest` |
| `POST /api/profiles/:id/fork` | `{version,policy}`; independent Profile with provenance |
| `PATCH /api/account` | Exactly `{nickname}`; unknown fields (including email) rejected |

Publishing the exact same slug/version/content is idempotent. A different payload for
an existing version is rejected. Cross-publisher slug takeover, cross-tenant resource
updates, forged managed metadata, invalid rules and invalid policy references fail.

Supported rule IR: DOMAIN, DOMAIN-SUFFIX, IP-CIDR, IP-CIDR6. 1–10,000 unique rules,
at most 1 MiB structured rules per package; existing final-output limit remains 4 MiB.
No regex, scripts, shell, MITM, external provider dependencies, or auto-update execution.
`no-resolve` is accepted only for IP rules. Source metadata pins commit/hash and
contains a complete license notice. Notices are also carried into rule-bearing
subscription exports and independent forks. Node-only subscriptions contain no rules.

## Curation and verification

`node scripts/curate-catalog.mjs <output-directory>` reads one pinned MIT-licensed
v2fly/domain-list-community commit via `gh`, creates reviewable publication JSON,
and has no publication side effects. It explicitly documents exclusions, recursively
expands the selected Bilibili CDN dependency and rejects unexpected syntax.

Initial selection:

| Package | Rules | Default | Deliberate scope |
| --- | ---: | --- | --- |
| douyin-direct | 71 | DIRECT | Douyin category, shopping/payment and related apps |
| bilibili-direct | 43 | DIRECT | Domestic sites/CDN; excludes international markers and game include |
| steam-cn-download | 11 | DIRECT | Selected domestic CDN only, not entire Steam/community |
| telegram-routing | 21 | Required | Domains only, not native-client IP networks |
| openai-routing | 19 | Required | Fixed domains; excludes telemetry and dynamic Azure regex |

Rules control routing; they do not guarantee app throughput, service unlocking,
completeness for hard-coded IPs, or provider account availability.

Verification commands:

```sh
cargo test --locked --lib --bin camofy-cloud
# TEST_DATABASE_URL must point to a disposable PostgreSQL, never production:
cargo test --locked --bin camofy-cloud catalog_end_to_end -- --ignored
```

The existing cloud/Agent integration test remains in CI. `scripts/verify-catalog.mjs`
requires an explicitly supplied test account, output directory and local Mihomo binary
(see its header). It creates temporary Profiles/identities, tests actual subscription
HTTP 200/304, expected domains/policies, notices and the cloud protection, invokes
Mihomo `-t`, starts an isolated no-TUN/no-DNS core and verifies real loopback proxy
requests match the expected domain and policy, checks Shadowrocket exports, and
removes only its created resources. The routing probe maps test domains to loopback;
it tests rule selection, not reachability or performance of the actual remote service.
It never binds a device or refreshes a paid upstream proxy. Shadowrocket output tests
do not substitute for physical iOS import/playback testing.

## Operations

Migration 0004 is additive. Back up PostgreSQL before deployment. Build the cloud
image off-server, publish and deploy by immutable digest, updating only the cloud
service. No router binary is changed for this feature. Retain the previous image and
database backup; downgrading SQLx migration-aware applications requires a coordinated
database/application rollback, not blindly starting an older binary against new schema.

An operator approves an existing publisher by inserting its user UUID and current
nickname into `catalog_publishers` in the same PostgreSQL instance. Do not store user
passwords, session tokens or private deployment details in the application repository.
Publishing uses that account's normal authenticated API session. Nickname changes
update its publicly displayed publisher name without changing immutable rule versions.

Deferred: automatic reviewed updates, bulk catalog pagination, package withdrawal
workflow, public submissions/moderation UI, arbitrary templates and large hosted rule
artifacts. The curated first release is not a general untrusted plugin execution system.
