ALTER TABLE agents ADD COLUMN next_decision_at TEXT;
ALTER TABLE agents ADD COLUMN budget_date TEXT;

UPDATE agents
SET next_decision_at = COALESCE(last_action_at, created_at),
    budget_date = substr(COALESCE(last_action_at, created_at), 1, 10);

CREATE INDEX agents_due_idx ON agents(run_id, next_decision_at, agent_id);
