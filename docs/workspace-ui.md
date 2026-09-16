# Cloud workspace UI

The workspace separates incoming data, reusable configuration and distribution:

- `/subscriptions`: upstream Clash YAML sources; source details expose refresh
  status, selected fetch proxy, schedule, referencing identities and cached YAML.
- `/profiles`: independent YAML fragments with content and referencing identities.
- `/identities`: ordered source/configuration compositions, per-binding enablement,
  neutral subscription links, merged previews, published history and settings.
- `/proxies`, `/devices`, `/tokens`: fetch providers, Agent reports and independent
  access credentials. These retain the existing backend authorization model.

Non-token resources have `/new`, `/:id`, and `/:id/edit` routes. Detail tabs and
collection searches use query parameters. `createBrowserRouter` supports hard
refresh, history navigation, direct links and unsaved-edit blockers. Backend SPA
fallback serves these routes; Vite's subscription proxy deliberately matches
`^/sub/`, not the `/subscriptions` workspace route.

## Implementation

- `web/src/CloudApp.tsx`: session, live updates, workspace shell and routes.
- `web/src/cloud/navigation.ts`: resource/route mapping and navigation metadata.
- `web/src/cloud/pages.tsx`: collections, details and page-level editors.
- `web/src/cloud/Forms.tsx`: login and resource-specific editing, with a frozen
  initial resource version so background updates cannot bypass conflict checks.
- `web/src/cloud/ui.tsx`: shared visual components and accessible native dialogs.
- `web/src/cloud.css`: responsive forest/paper visual system, no external font
  request, SVG icons, bounded code previews and table-local horizontal scrolling.

`GET /api/profiles/:id/content` requires an authenticated owning user. It returns
the cached content/version/last_fetch, 409 when missing, and 404 for foreign or
non-profile IDs. It does not refresh upstream data. Responses inherit `no-store`.

## Verification

2026-09-16: production frontend build and ESLint pass; 19 Rust unit tests and
disposable-PostgreSQL integration pass; Clippy passes with warnings denied.
Integration covers cached content, authentication, tenant isolation and no fetch
side effect. Browser verification uses isolated local fixture data, not production
mutations: 1440x1000 desktop and 390x844 mobile, new/save/detail, unsaved-edit
confirmation, identity-local toggle and publication, preview/history, source
snapshot, refresh persistence and back navigation. Narrow-screen document width
equals viewport width; only the table content scrolls horizontally.

Deploy by local image build, push to `registry.example.com`, pin the digest under
`private-deployment/ym` using `registry-mirror.example.com`, then use its bounded deployment helper.
Never copy app source to that deployment repository or build on the server.
