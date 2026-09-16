CREATE TABLE platform_proxies (
    id UUID PRIMARY KEY,
    data JSONB NOT NULL,
    version BIGINT NOT NULL DEFAULT 1,
    updated_by UUID REFERENCES users(id) ON DELETE SET NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE subscription_egress_policy (
    singleton BOOLEAN PRIMARY KEY DEFAULT true CHECK (singleton),
    proxy_id UUID REFERENCES platform_proxies(id) ON DELETE RESTRICT,
    version BIGINT NOT NULL DEFAULT 1,
    updated_by UUID REFERENCES users(id) ON DELETE SET NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
INSERT INTO subscription_egress_policy(singleton) VALUES(true);
CREATE TABLE admin_audit_events (
    id BIGSERIAL PRIMARY KEY,
    actor_id UUID REFERENCES users(id) ON DELETE SET NULL,
    action TEXT NOT NULL,
    object_id UUID,
    version BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
