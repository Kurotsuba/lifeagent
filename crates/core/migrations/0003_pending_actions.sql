-- Tool calls the conversation model made that only run once the user approves them in the app
-- (docs/DECISIONS.md, "Approval for sensitive tool calls"). The model's request is stored as
-- is; `preview_json` is what the UI shows, as data it words itself.

CREATE TABLE pending_actions (
  id              TEXT PRIMARY KEY,          -- UUID v7
  tool_name       TEXT NOT NULL,
  arguments_json  TEXT NOT NULL,
  preview_json    TEXT NOT NULL,             -- {"kind": ..., ...}, see core approvals.rs
  status          TEXT NOT NULL,             -- 'pending' | 'approved' | 'rejected' | 'failed'
  result_summary  TEXT,                      -- the tool's summary, or why it failed
  created_at      TEXT NOT NULL,
  resolved_at     TEXT
);
CREATE INDEX pending_actions_created_at ON pending_actions(created_at);
