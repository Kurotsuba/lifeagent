-- The agent activity log (docs/AUDIT_LOG_SPEC.md §4): one run per agent entry point (chat
-- turn, plan, approval), with its steps in order. Append-only by code; pruned after 90 days.

CREATE TABLE agent_runs (
  id             TEXT PRIMARY KEY,      -- UUID v7, so ordering by id is ordering by start
  trigger        TEXT NOT NULL,         -- 'chat_turn' | 'plan_scheduled' | 'plan_manual' | ...
  actor          TEXT NOT NULL,         -- 'user' | 'scheduler'
  status         TEXT NOT NULL,         -- 'running' | 'succeeded' | 'needs_approval' | 'failed'
  input_excerpt  TEXT,                  -- the user's message or request, at most 200 chars
  outcome_json   TEXT,                  -- counts for the list view, set when the run ends
  error_code     TEXT,                  -- CoreError::code() when failed
  logical_date   TEXT NOT NULL,
  started_at     TEXT NOT NULL,
  finished_at    TEXT
);
CREATE INDEX agent_runs_started_at ON agent_runs(started_at);

CREATE TABLE agent_steps (
  id           TEXT PRIMARY KEY,        -- UUID v7
  run_id       TEXT NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
  seq          BIGINT GENERATED ALWAYS AS IDENTITY,  -- order within the run
  kind         TEXT NOT NULL,           -- 'model_call' | 'tool_call' | 'guardrail' | ...
  detail_json  TEXT NOT NULL,
  created_at   TEXT NOT NULL
);
CREATE INDEX agent_steps_run ON agent_steps(run_id, seq);

-- The run that queued the action, so the log links "the agent asked" to "you decided".
ALTER TABLE pending_actions ADD COLUMN run_id TEXT;
