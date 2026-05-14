-- Persistent salience score for episodic events.
--
-- The architecture's write-path step 2 ("score salience") and step 4
-- ("salience gate for NC promotion") both reference this. v0 ships:
--   - column with sane default (0.5) so existing rows don't violate NOT NULL
--   - the ripple consolidator bumps the salience of fit-events so the
--     eventual dream cycle has a real signal to prioritize
--
-- Range is [0.0, 1.0]; the storage layer clamps. The default of 0.5 means
-- "neither suppressed nor reinforced" — anything below is decay/suppression
-- territory, anything above is consolidator-tagged.

ALTER TABLE episodic
    ADD COLUMN salience real NOT NULL DEFAULT 0.5;

ALTER TABLE episodic
    ADD CONSTRAINT episodic_salience_range CHECK (salience >= 0.0 AND salience <= 1.0);

-- Range index for the long-sleep decay sweep ("find low-salience events").
CREATE INDEX episodic_tenant_salience_idx ON episodic (tenant_id, salience);
