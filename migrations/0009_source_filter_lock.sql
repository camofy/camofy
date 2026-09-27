ALTER TABLE revisions ADD COLUMN source_filter_lock JSONB NOT NULL DEFAULT '[]';
