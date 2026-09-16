# Canonical cloud origin

The public cloud and the Agent's default authorization server use `https://camofy.app`.
Self-hosted installations set `CAMOFY_PUBLIC_URL` to their own HTTPS origin.

Changing the origin preserves account data, device credentials and subscription tokens.
Resource responses normalize persisted subscription links to the current public origin.
Re-save each identity after migration to publish its updated system DIRECT rule; changing
the URL alone does not rewrite immutable configuration revisions.

For a domain cutover, first make both origins available and update bound Agents to
the new origin (Agents deliberately reject redirects). Then redirect every old-domain
request to the new origin with HTTP 308, preserving the complete path and query string.
Subscription clients must support redirects or update their stored URL. Keep the old
DNS record and TLS site alive for compatibility until explicitly retired.

`CAMOFY_LEGACY_ORIGINS` is an optional, explicit HTTPS origin allowlist for deployments
that temporarily serve old browser sessions without redirecting API requests. It is
not needed after the full redirect cutover. Cookies are not shared across domains;
users sign in again on the new domain.

Update existing devices' cloud origin while retaining their device credentials and
identity binding. Do not enable TUN or change traffic interception during migration.

Deployment records in this repository use anonymized example registry names, operator
accounts and private deployment paths. Actual infrastructure credentials and rollback
backups belong outside this source repository. Git history rewriting changes commit
IDs: remote repositories, forks and hosting caches require separate cleanup, and old
clones must not be merged back into the sanitized history.

## Verification (2026-09-16)

- Cloudflare-proxied DNS and valid origin/public HTTPS verified.
- All five existing identity links use the canonical origin. Each published YAML
  contains its DIRECT protection; the legacy URL follows a 308 to identical YAML.
- Root, detail, API and subscription redirects preserve paths and query strings.
- The bound router uses the new origin, Mihomo remains running, TUN remains off,
  and the routing table, policy rules and firewall rules are unchanged.
- 24 unit tests, the database/Agent integration test and the Agent rollback/offline
  recovery test passed; Clippy completed without warnings.
- Builds ran on the workstation only. Unrelated service health and container
  identities were verified unchanged throughout deployment.
- Local project contents, all main-repository Git objects and submodule Git objects
  passed the privacy keyword audit. Prior history and generated diagnostic files
  were archived outside the source repository; no remote force push was performed.
