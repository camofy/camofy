CREATE TABLE users (
    id UUID PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    password TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE sessions (
    hash TEXT PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX sessions_expiry ON sessions(expires_at);
CREATE TABLE resources (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('proxy', 'profile', 'bundle', 'device')),
    data JSONB NOT NULL,
    version BIGINT NOT NULL DEFAULT 1,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (user_id, id)
);
CREATE INDEX resources_owner_kind ON resources(user_id, kind);
CREATE TABLE revisions (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL,
    bundle_id UUID NOT NULL,
    artifacts JSONB NOT NULL,
    selections JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (user_id, bundle_id) REFERENCES resources(user_id, id) ON DELETE CASCADE
);
CREATE INDEX revisions_bundle ON revisions(bundle_id, created_at DESC);
CREATE TABLE access_tokens (
    hash TEXT PRIMARY KEY,
    user_id UUID NOT NULL,
    bundle_id UUID NOT NULL,
    device_id UUID,
    label TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (user_id, bundle_id) REFERENCES resources(user_id, id) ON DELETE CASCADE,
    FOREIGN KEY (user_id, device_id) REFERENCES resources(user_id, id) ON DELETE CASCADE
);
CREATE INDEX tokens_owner ON access_tokens(user_id);
CREATE TABLE fetch_jobs (
    profile_id UUID PRIMARY KEY REFERENCES resources(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    next_run TIMESTAMPTZ NOT NULL DEFAULT now(),
    leased_until TIMESTAMPTZ NOT NULL DEFAULT '-infinity',
    claim UUID
);
CREATE INDEX fetch_jobs_due ON fetch_jobs(next_run, leased_until);
CREATE TABLE rate_limits (
    key TEXT PRIMARY KEY,
    count INTEGER NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL
);
