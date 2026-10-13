-- Proactive check-ins the scheduler raises on its own (docs/DECISIONS.md, "Proactive
-- follow-ups"): a plugin's check finds something worth asking about (a planned meal not logged
-- by its check time, a day far off its targets), and the app shows it with one-tap answers.
-- At most one per (logical_date, key), so a check-in is never raised twice in a day.

CREATE TABLE follow_ups (
  id            TEXT PRIMARY KEY,          -- UUID v7
  logical_date  TEXT NOT NULL,
  key           TEXT NOT NULL,             -- e.g. 'diet_meal_missed:lunch'
  kind          TEXT NOT NULL,             -- e.g. 'diet_meal_missed', worded by the UI
  params_json   TEXT NOT NULL,             -- data the UI words it with
  actions_json  TEXT NOT NULL,             -- answer kinds offered, in order
  reason        TEXT NOT NULL,             -- English, for the activity log
  instruction   TEXT,                      -- for 'core_adjust_tomorrow': the planner request
  status        TEXT NOT NULL,             -- 'open' | 'answered' | 'dismissed' | 'cleared'
  answer        TEXT,                      -- the answer kind the user picked
  result_json   TEXT,                      -- what the answer did (a ToolSummary)
  run_id        TEXT,                      -- the activity-log run that raised it
  created_at    TEXT NOT NULL,
  resolved_at   TEXT,
  UNIQUE (logical_date, key)
);
CREATE INDEX follow_ups_logical_date ON follow_ups(logical_date);
