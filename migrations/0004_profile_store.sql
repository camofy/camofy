ALTER TABLE users ADD COLUMN nickname TEXT NOT NULL DEFAULT '新用户';
CREATE TABLE catalog_publishers (
    user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    display_name TEXT NOT NULL CHECK (length(display_name) BETWEEN 1 AND 80)
);
CREATE TABLE catalog_packages (
    slug TEXT PRIMARY KEY,
    owner_id UUID REFERENCES users(id) ON DELETE SET NULL,
    publisher TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE catalog_artifacts (
    hash TEXT PRIMARY KEY,
    content JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE catalog_versions (
    id UUID PRIMARY KEY,
    package_slug TEXT NOT NULL REFERENCES catalog_packages(slug),
    version TEXT NOT NULL,
    manifest JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(package_slug, version)
);
CREATE TABLE catalog_version_artifacts (
    version_id UUID NOT NULL REFERENCES catalog_versions(id),
    artifact_hash TEXT NOT NULL REFERENCES catalog_artifacts(hash),
    role TEXT NOT NULL,
    PRIMARY KEY(version_id, role)
);
ALTER TABLE revisions ADD COLUMN catalog_lock JSONB NOT NULL DEFAULT '[]';
CREATE INDEX catalog_versions_package ON catalog_versions(package_slug, created_at DESC);
