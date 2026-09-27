ALTER TABLE revisions ADD COLUMN capability_lock JSONB NOT NULL DEFAULT '[]';
