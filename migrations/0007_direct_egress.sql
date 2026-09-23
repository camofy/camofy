ALTER TABLE subscription_egress_policy
    ADD COLUMN direct BOOLEAN NOT NULL DEFAULT false,
    ADD CONSTRAINT subscription_egress_direct_without_proxy
        CHECK (NOT direct OR proxy_id IS NULL);
