-- Metadata is independent of device configuration hashes. NULL marks legacy revisions.
ALTER TABLE revisions ADD COLUMN usage_sources JSONB;
ALTER TABLE fetch_jobs ADD COLUMN reason TEXT NOT NULL DEFAULT 'scheduled';
ALTER TABLE fetch_jobs ADD COLUMN request_id UUID NOT NULL DEFAULT gen_random_uuid();
CREATE TABLE refresh_history (
    id BIGSERIAL PRIMARY KEY,
    user_id UUID NOT NULL,
    profile_id UUID NOT NULL,
    claim UUID NOT NULL UNIQUE,
    started_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    finished_at TIMESTAMPTZ,
    duration_ms BIGINT,
    reason TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'running',
    error_code TEXT,
    message TEXT,
    usage_status TEXT,
    FOREIGN KEY (user_id, profile_id) REFERENCES resources(user_id, id) ON DELETE CASCADE
);
CREATE INDEX refresh_history_profile ON refresh_history(user_id, profile_id, id DESC);
CREATE INDEX refresh_history_expiry ON refresh_history(started_at);
