CREATE TABLE store_identity (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    database_id TEXT NOT NULL UNIQUE,
    network TEXT NOT NULL CHECK (length(trim(network)) > 0),
    surfnet_id TEXT NOT NULL CHECK (length(trim(surfnet_id)) > 0),
    created_at TEXT NOT NULL
);

CREATE TABLE runs (
    run_id TEXT PRIMARY KEY,
    status TEXT NOT NULL CHECK (status IN ('active', 'completed', 'cancelled', 'failed')),
    model_version TEXT,
    config_json TEXT,
    config_hash TEXT,
    seed_hash TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE agents (
    agent_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES runs(run_id) ON DELETE RESTRICT,
    next_sequence INTEGER NOT NULL CHECK (next_sequence >= 0),
    session_state TEXT NOT NULL CHECK (
        session_state IN ('dormant', 'active', 'transacting', 'holding', 'cooling_down')
    ),
    last_action_at TEXT,
    remaining_daily_budget INTEGER NOT NULL CHECK (remaining_daily_budget >= 0),
    model_version TEXT NOT NULL,
    snapshot_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (agent_id, run_id)
);

CREATE TABLE actions (
    action_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES runs(run_id) ON DELETE RESTRICT,
    agent_id TEXT NOT NULL,
    sequence INTEGER NOT NULL CHECK (sequence >= 0),
    model_version TEXT NOT NULL,
    action_kind TEXT NOT NULL,
    scheduled_at TEXT NOT NULL,
    max_fee_lamports INTEGER NOT NULL CHECK (max_fee_lamports >= 0),
    state TEXT NOT NULL CHECK (
        state IN (
            'planned', 'simulated', 'submitted', 'confirmed', 'rejected',
            'failed', 'expired', 'unknown', 'orphaned', 'cancelled'
        )
    ),
    action_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE (run_id, agent_id, sequence, model_version),
    FOREIGN KEY (agent_id, run_id) REFERENCES agents(agent_id, run_id) ON DELETE RESTRICT
);

CREATE TABLE leases (
    lease_id TEXT PRIMARY KEY,
    action_id TEXT NOT NULL REFERENCES actions(action_id) ON DELETE RESTRICT,
    worker_id TEXT NOT NULL CHECK (length(trim(worker_id)) > 0),
    acquired_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    released_at TEXT,
    release_reason TEXT,
    CHECK (expires_at > acquired_at),
    CHECK (
        (released_at IS NULL AND release_reason IS NULL)
        OR (released_at IS NOT NULL AND release_reason IS NOT NULL)
    )
);

CREATE UNIQUE INDEX leases_one_active_per_action
    ON leases(action_id) WHERE released_at IS NULL;

CREATE TABLE prepared_transactions (
    action_id TEXT PRIMARY KEY REFERENCES actions(action_id) ON DELETE RESTRICT,
    signature TEXT NOT NULL UNIQUE CHECK (length(trim(signature)) > 0),
    transaction_bytes BLOB NOT NULL CHECK (length(transaction_bytes) > 0),
    transaction_hash TEXT NOT NULL UNIQUE,
    recent_blockhash TEXT NOT NULL CHECK (length(trim(recent_blockhash)) > 0),
    last_valid_block_height INTEGER NOT NULL CHECK (last_valid_block_height >= 0),
    expectations_json TEXT NOT NULL,
    prepared_at TEXT NOT NULL,
    UNIQUE (action_id, signature)
);

CREATE TABLE simulations (
    simulation_id INTEGER PRIMARY KEY AUTOINCREMENT,
    action_id TEXT NOT NULL REFERENCES actions(action_id) ON DELETE RESTRICT,
    succeeded INTEGER NOT NULL CHECK (succeeded IN (0, 1)),
    units_consumed INTEGER CHECK (units_consumed >= 0),
    receipt_json TEXT NOT NULL,
    receipt_hash TEXT NOT NULL,
    recorded_at TEXT NOT NULL,
    UNIQUE (action_id, receipt_hash)
);

CREATE TABLE submissions (
    action_id TEXT PRIMARY KEY REFERENCES actions(action_id) ON DELETE RESTRICT,
    signature TEXT NOT NULL UNIQUE,
    submitted_at TEXT NOT NULL,
    UNIQUE (action_id, signature),
    FOREIGN KEY (action_id, signature)
        REFERENCES prepared_transactions(action_id, signature) ON DELETE RESTRICT
);

CREATE TABLE receipts (
    receipt_id INTEGER PRIMARY KEY AUTOINCREMENT,
    action_id TEXT NOT NULL REFERENCES actions(action_id) ON DELETE RESTRICT,
    signature TEXT NOT NULL,
    confirmation_status TEXT NOT NULL CHECK (
        confirmation_status IN ('missing', 'processed', 'confirmed', 'finalized', 'failed')
    ),
    slot INTEGER CHECK (slot >= 0),
    postconditions_met INTEGER NOT NULL CHECK (postconditions_met IN (0, 1)),
    receipt_json TEXT NOT NULL,
    receipt_hash TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    UNIQUE (action_id, receipt_hash),
    FOREIGN KEY (action_id, signature)
        REFERENCES submissions(action_id, signature) ON DELETE RESTRICT
);

CREATE TABLE action_events (
    event_id INTEGER PRIMARY KEY AUTOINCREMENT,
    action_id TEXT NOT NULL REFERENCES actions(action_id) ON DELETE RESTRICT,
    event_kind TEXT NOT NULL CHECK (length(trim(event_kind)) > 0),
    from_state TEXT,
    to_state TEXT,
    occurred_at TEXT NOT NULL,
    detail TEXT,
    payload_json TEXT
);

CREATE TABLE traces (
    trace_id INTEGER PRIMARY KEY AUTOINCREMENT,
    schema_version INTEGER NOT NULL CHECK (schema_version > 0),
    run_id TEXT NOT NULL REFERENCES runs(run_id) ON DELETE RESTRICT,
    agent_id TEXT NOT NULL REFERENCES agents(agent_id) ON DELETE RESTRICT,
    action_id TEXT NOT NULL REFERENCES actions(action_id) ON DELETE RESTRICT,
    event_json TEXT NOT NULL,
    event_hash TEXT NOT NULL UNIQUE,
    appended_at TEXT NOT NULL
);

CREATE INDEX actions_due_idx ON actions(state, scheduled_at, action_id);
CREATE INDEX actions_reconcile_idx ON actions(state, updated_at, action_id);
CREATE INDEX leases_action_history_idx ON leases(action_id, acquired_at, lease_id);
CREATE INDEX simulations_action_idx ON simulations(action_id, simulation_id);
CREATE INDEX receipts_action_idx ON receipts(action_id, receipt_id);
CREATE INDEX action_events_order_idx ON action_events(action_id, event_id);
CREATE INDEX traces_run_order_idx ON traces(run_id, trace_id);
