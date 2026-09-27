CREATE TABLE assistant_sessions (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    scope_kind TEXT NOT NULL DEFAULT 'profile',
    profile_id UUID REFERENCES resources(id) ON DELETE CASCADE,
    state JSONB NOT NULL,
    current_draft UUID,
    status TEXT NOT NULL DEFAULT 'idle' CHECK (status IN ('idle', 'running')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (scope_kind <> 'profile' OR profile_id IS NOT NULL)
);
CREATE INDEX assistant_sessions_owner ON assistant_sessions(user_id, profile_id, updated_at DESC);

CREATE TABLE assistant_drafts (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    session_id UUID NOT NULL REFERENCES assistant_sessions(id) ON DELETE CASCADE,
    profile_id UUID NOT NULL REFERENCES resources(id) ON DELETE CASCADE,
    parent_id UUID REFERENCES assistant_drafts(id),
    base_version BIGINT NOT NULL,
    base_hash TEXT NOT NULL,
    base_content JSONB NOT NULL,
    content JSONB NOT NULL,
    content_hash TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft', 'superseded', 'committed')),
    committed_result JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX assistant_drafts_session ON assistant_drafts(session_id, created_at DESC);

CREATE TABLE assistant_runs (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    session_id UUID NOT NULL REFERENCES assistant_sessions(id) ON DELETE CASCADE,
    status TEXT NOT NULL CHECK (status IN ('running', 'completed', 'failed')),
    started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at TIMESTAMPTZ,
    error_code TEXT
);
CREATE INDEX assistant_runs_session ON assistant_runs(session_id, started_at DESC);

CREATE TABLE assistant_events (
    id BIGSERIAL PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    session_id UUID NOT NULL REFERENCES assistant_sessions(id) ON DELETE CASCADE,
    event JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX assistant_events_session ON assistant_events(session_id, id);
