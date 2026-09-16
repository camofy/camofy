# Cloud-hosted installation and release distribution

`GET /install.sh` serves the installer embedded in the cloud binary, not a remote
repository file. Its default cloud origin comes from `CAMOFY_PUBLIC_URL`.

Public endpoints (GET and HEAD only):

- `/github/repos/camofy/camofy/releases/latest`: cached release metadata.
- `/github/repos/MetaCubeX/mihomo/releases/latest`: cached core metadata.
- `/github/repos/MetaCubeX/meta-rules-dat/releases/latest`: cached rules metadata.
- `/github/<approved-owner>/<approved-repo>/releases/download/<tag>/<asset>`:
  streaming asset response, with single byte-range support.
- `/github/<approved-owner>/<approved-repo>/releases/latest/download/<asset>`:
  resolve the GitHub redirect on the server and stream the actual file.
- `/downloads/manifest/amd64` and `/downloads/manifest/armv7`: two text records
  (`kind URL SHA256`) containing tag-pinned Agent and core downloads.

Metadata `assets[].browser_download_url` always points back to the configured cloud.
Clients never receive an upstream redirect, GitHub cookies or API credentials.
Caller authorization headers, cookies and query tokens are not forwarded. Query
parameters are rejected; only the three exact repositories above are accessible.
Redirect destinations must be HTTPS on approved GitHub release hosts. This is not
a general GitHub/HTTP proxy; raw source files, APIs other than latest releases,
uploads and other repositories are deliberately unavailable.

Each process has four concurrent upstream transfer slots. Asset responses stream
without whole-file buffering or a writable cache volume and are bounded at 256 MiB
and five minutes. Metadata is limited to 2 MiB, cached for five minutes and fetched
with a 25-second deadline. Optional `CAMOFY_GITHUB_TOKEN` is an operator-provided
read-only API token, never a caller-provided token. Public metadata is available
without it subject to GitHub's unauthenticated rate limit. Do not put tokens in URLs.

## Publishing an installable version

The current Agent must be published by `.github/workflows/release.yml` as a stable
GitHub Release, with both of these assets:

- `camofy-agent-x86_64-unknown-linux-musl.tar.gz`
- `camofy-agent-armv7-unknown-linux-musleabihf.tar.gz`

Each tarball contains only `camofy-agent`; the workflow also produces SHA256 sidecars.
The installer manifest requires GitHub's `assets[].digest` SHA256 metadata for both
the Agent archive and the selected Mihomo gzip. Missing assets/digests return 503;
the old `camofy-linux-*` monolith is never silently installed as a new Agent.
Release publication is separate from deploying the cloud. Do not force-push history
or publish a GitHub release implicitly as part of server deployment.

## Installer behavior

The script supports only Linux amd64 and ARMv7, with an explicit error otherwise.
It defaults to `/jffs/camofy` on writable JFFS routers, otherwise
`$HOME/.local/share/camofy`. `--data-root`, `--cloud` (HTTPS), `--listen` (IPv4:port),
`--no-start` and `--plan` are supported. Existing target directories and existing
Camofy router boot hooks cause refusal before downloading or changing anything.

Download/verification/decompression happens in a private temporary directory before
creating the installation directory. Archive entries are restricted and the single
executable is extracted to stdout. Failed downloads and checksum mismatches leave
existing installation data untouched. A new installation sets `dns_redirect=false`
and a durable stopped state: Mihomo will not start or take over traffic automatically.
The initial local UI binds to the router LAN IPv4 when available, otherwise loopback.
Only explicitly selecting `--listen 0.0.0.0:3000` exposes all IPv4 interfaces.

On JFFS routers, a new scoped boot hook runs the Agent supervisor. Other Linux hosts
must register `start-agent.sh` with their service manager themselves. The script is
not an in-place upgrade or legacy configuration migration tool.

Run `scripts/test-installer.sh` in a disposable Linux container. It uses synthetic
downloads and never accesses a real router. `--plan` is safe for inspecting a target.

## Deployment verification (2026-09-16)

The cloud endpoints are deployed. The served installer SHA256 matches the embedded
source. All 24 unit tests, the database/Agent integration test, Clippy and the
isolated installer tests pass. Live verification downloaded the complete 21,108,932
byte Mihomo ARMv7 archive through the cloud and matched GitHub's SHA256 digest;
single-range requests and HEAD also passed without exposing upstream redirects.
Repository, method, query-token and architecture restrictions were exercised.
The live `--plan` command made no installation changes. Existing device operation
and unrelated service health were verified after deployment.

At verification time GitHub's latest Camofy release was `v0.0.2`, containing the old
monolith. The installation manifest therefore correctly returned 503. Publishing
the new Agent via GitHub Actions remains a required release step; the presence of
`/install.sh` alone does not mean installation can complete yet.
