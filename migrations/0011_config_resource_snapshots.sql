-- Rule resources are immutable within a revision/target/compiler input. Only
-- this new cache table is written; existing identities and revisions are unchanged.
CREATE TABLE IF NOT EXISTS config_resource_snapshots (
    cache_key TEXT PRIMARY KEY CHECK (length(cache_key) = 64),
    user_id UUID NOT NULL,
    bundle_id UUID NOT NULL,
    revision_id UUID REFERENCES revisions(id) ON DELETE CASCADE,
    sealed JSONB,
    claim UUID NOT NULL,
    lease_until TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (user_id, bundle_id) REFERENCES resources(user_id, id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS config_resource_snapshots_scope
    ON config_resource_snapshots(user_id, bundle_id, revision_id);
CREATE INDEX IF NOT EXISTS config_resource_snapshots_pending
    ON config_resource_snapshots(lease_until) WHERE sealed IS NULL;
