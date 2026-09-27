# Camofy Agent · independent Profile editing

The first release edits only a tenant's independent `overlay` Profile `content`.
Subscription sources and store-managed Profiles are never writable through the
agent. Future tools can reuse the session, run, event, authorization and audit
infrastructure without giving the model database, shell, or arbitrary HTTP access.

## Runtime configuration

Set these **only on the cloud server**, not in the public repository or browser:

- `CAMOFY_AI_BASE_URL`: HTTPS Responses API base URL ending in `/v1`.
- `CAMOFY_AI_API_KEY`: gateway bearer secret.
- `CAMOFY_AI_PORTAL_URL`: optional HTTPS target for the “more models” link.

The model is fixed in code to `gpt-6-luna` with `reasoning.effort=medium`.
Requests use `POST /responses`, streaming, `store=false`, and
`parallel_tool_calls=false`. Unavailable models or gateways return an error;
there is no silent fallback. The cloud configuration endpoint exposes only
availability, model, effort and the optional portal URL.

## Editing transaction

1. The user opens AI editing on an independent Profile and starts a scoped
   session. Each tool call checks the tenant and Profile scope again.
2. `profile_read` returns a bounded, line-numbered view and an opaque
   `base_ref`. Known credential and URL fields are masked. A read can target the
   live Profile or the current draft.
3. `profile_replace` requires unique literal matches against the read version.
   Secret-containing lines cannot be touched or inserted. It creates an
   encrypted, immutable draft; it does not publish anything. A later edit
   supersedes the earlier draft.
4. The first-party UI displays before/after content, validation errors, and
   affected identities. The model's `profile_commit` tool can only request
   human review, never authorize a commit.
5. The user confirms the exact preview hash. In one tenant transaction, the
   server checks the source version, re-renders every affected identity, writes
   Profile content, publishes those identities and marks the draft committed.
   Any failure rolls everything back. Repeating a successful commit returns
   its recorded result without creating new revisions. Device application is
   reported separately by the normal device feedback path.

Drafts, model context and conversation events are encrypted with the existing
cloud vault. Runs and events are auditable without logging raw Profile content
or gateway credentials. Per-user rate limits, a global model concurrency cap,
bounded reads/replacements/context, a 180-second turn deadline and 30-day idle
session retention limit resource usage.

## Verification

Run `cargo test --bin camofy-cloud assistant::tests` for the pure replacement
and scope tests. For the database and fake Responses-stream integration tests,
set `TEST_DATABASE_URL` to an **explicitly disposable** PostgreSQL instance,
then run:

```text
cargo test --bin camofy-cloud assistant::tests -- --include-ignored
```

Run `bun run build` and `bun run lint` in `web/`. The `design` Vite mode has
local-only synthetic Agent responses for browser and mobile layout checks; it
does not contact the real gateway.

An additional ignored test, `real_responses_to_draft_end_to_end`, uses a
synthetic local Profile and the configured gateway to verify streamed function
calling and draft creation. It requires a funded key and the same disposable
database, and must never be pointed at production data.
