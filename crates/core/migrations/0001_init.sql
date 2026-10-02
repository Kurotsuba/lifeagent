-- Core schema (Postgres). See docs/PHASE1_SPEC.md §5. Plugins add their own tables via plugin
-- migrations (§7.6), prefixed with their plugin id.
--
-- Ported from the Phase 1 SQLite schema with the same columns and meanings: instants and dates
-- stay RFC 3339 / ISO TEXT, JSON stays TEXT, INTEGER becomes BIGINT (Rust i64) and flags stay
-- 0/1 integers. `seq` replaces SQLite's implicit rowid as the insertion-order tie-breaker.

CREATE TABLE settings (
  key         TEXT PRIMARY KEY,
  value_json  TEXT NOT NULL,
  updated_at  TEXT NOT NULL
);

CREATE TABLE profile (
  id          BIGINT PRIMARY KEY CHECK (id = 1),    -- single row
  data_json   TEXT NOT NULL,                        -- Profile struct
  updated_at  TEXT NOT NULL
);

CREATE TABLE goals (
  id          TEXT PRIMARY KEY,
  domain      TEXT NOT NULL,          -- 'body' in Phase 1
  kind        TEXT NOT NULL,          -- 'weight'
  params_json TEXT NOT NULL,
  active      BIGINT NOT NULL DEFAULT 1,
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL
);

CREATE TABLE events (
  id            TEXT PRIMARY KEY,
  seq           BIGINT GENERATED ALWAYS AS IDENTITY,
  event_type    TEXT NOT NULL,        -- 'diet.meal_logged', 'body.weight_logged', ...
  occurred_at   TEXT NOT NULL,
  logical_date  TEXT NOT NULL,
  payload_json  TEXT NOT NULL,
  source        TEXT NOT NULL,        -- 'chat' | 'ui' | 'system'
  created_at    TEXT NOT NULL,
  updated_at    TEXT NOT NULL,
  deleted_at    TEXT
);
CREATE INDEX idx_events_type_date ON events(event_type, logical_date) WHERE deleted_at IS NULL;

CREATE TABLE plans (
  plan_date     TEXT NOT NULL,
  version       BIGINT NOT NULL,
  status        TEXT NOT NULL,        -- 'proposed' | 'accepted'
  trigger       TEXT NOT NULL,        -- 'schedule' | 'manual' | 'revision'
  instruction   TEXT,                 -- revision instruction, if any
  content_json  TEXT NOT NULL,        -- PlanContent
  warnings_json TEXT NOT NULL,        -- Vec<String>
  created_at    TEXT NOT NULL,
  PRIMARY KEY (plan_date, version)
);

CREATE TABLE memory_notes (
  id          TEXT PRIMARY KEY,
  seq         BIGINT GENERATED ALWAYS AS IDENTITY,
  note        TEXT NOT NULL,
  created_at  TEXT NOT NULL,
  deleted_at  TEXT
);

CREATE TABLE messages (
  id              TEXT PRIMARY KEY,
  seq             BIGINT GENERATED ALWAYS AS IDENTITY,
  role            TEXT NOT NULL,      -- 'user' | 'assistant' | 'tool'
  content         TEXT,
  tool_calls_json TEXT,               -- for assistant messages with tool calls
  tool_call_id    TEXT,               -- for tool messages
  tool_name       TEXT,
  created_at      TEXT NOT NULL
);
CREATE INDEX idx_messages_seq ON messages(seq);

CREATE TABLE llm_usage (
  id                 TEXT PRIMARY KEY,
  role               TEXT NOT NULL,
  provider_id        TEXT NOT NULL,
  model              TEXT NOT NULL,
  prompt_tokens      BIGINT NOT NULL,
  completion_tokens  BIGINT NOT NULL,
  latency_ms         BIGINT NOT NULL,
  ok                 BIGINT NOT NULL,
  created_at         TEXT NOT NULL
);

CREATE TABLE plugin_migrations (
  plugin_id   TEXT NOT NULL,
  version     BIGINT NOT NULL,
  applied_at  TEXT NOT NULL,
  PRIMARY KEY (plugin_id, version)
);
