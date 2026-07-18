CREATE TABLE action_deferrals (
    action_id TEXT PRIMARY KEY REFERENCES actions(action_id) ON DELETE RESTRICT,
    eligible_at TEXT NOT NULL,
    reason TEXT NOT NULL CHECK (length(trim(reason)) > 0),
    deferred_at TEXT NOT NULL,
    CHECK (eligible_at > deferred_at)
);

CREATE INDEX action_deferrals_due_idx ON action_deferrals(eligible_at, action_id);
