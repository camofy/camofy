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

Build and lint the frontend, test authenticated resource access with isolated
fixtures, and inspect desktop/mobile screenshots. Verify navigation, tab/query
persistence, unsaved edits, loading/error states, keyboard focus and overflow.
Keep real accounts and deployment records out of fixtures and public documents.
