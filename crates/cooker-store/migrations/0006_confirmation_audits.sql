CREATE TABLE confirmation_audits (
    audit_id INTEGER PRIMARY KEY AUTOINCREMENT,
    action_id TEXT NOT NULL REFERENCES actions(action_id) ON DELETE RESTRICT,
    signature TEXT NOT NULL,
    outcome TEXT NOT NULL CHECK (outcome IN ('verified', 'orphaned')),
    confirmation_status TEXT NOT NULL CHECK (
        confirmation_status IN ('missing', 'processed', 'confirmed', 'finalized', 'failed')
    ),
    slot INTEGER CHECK (slot >= 0),
    postconditions_met INTEGER NOT NULL CHECK (postconditions_met IN (0, 1)),
    receipt_json TEXT NOT NULL,
    receipt_hash TEXT NOT NULL,
    audited_at TEXT NOT NULL,
    UNIQUE (action_id, receipt_hash, audited_at),
    FOREIGN KEY (action_id, signature)
        REFERENCES submissions(action_id, signature) ON DELETE RESTRICT
);

CREATE INDEX confirmation_audits_action_time_idx
    ON confirmation_audits(action_id, audited_at, audit_id);

CREATE TRIGGER confirmation_audits_reject_update
BEFORE UPDATE ON confirmation_audits
BEGIN
    SELECT RAISE(ABORT, 'confirmation audits are immutable');
END;

CREATE TRIGGER confirmation_audits_reject_delete
BEFORE DELETE ON confirmation_audits
BEGIN
    SELECT RAISE(ABORT, 'confirmation audits are immutable');
END;
