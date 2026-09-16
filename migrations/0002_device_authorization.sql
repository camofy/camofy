CREATE TABLE device_authorizations (
    device_hash TEXT PRIMARY KEY,
    user_code_hash TEXT NOT NULL UNIQUE,
    device_name TEXT NOT NULL,
    return_uri TEXT NOT NULL,
    state TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'approved', 'denied')),
    user_id UUID REFERENCES users(id) ON DELETE CASCADE,
    bundle_id UUID,
    expires_at TIMESTAMPTZ NOT NULL,
    next_poll TIMESTAMPTZ NOT NULL DEFAULT now(),
    poll_interval INTEGER NOT NULL DEFAULT 5,
    FOREIGN KEY (user_id, bundle_id) REFERENCES resources(user_id, id) ON DELETE CASCADE
);
CREATE INDEX device_authorizations_expiry ON device_authorizations(expires_at);
