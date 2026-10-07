-- Tool summaries leave Rust as data the UI words in its own language
-- (docs/AUDIT_LOG_SPEC.md §3.6). An approved action's summary goes here as
-- {"kind": ..., ...}; `result_summary` keeps only why an approved action failed (and the
-- text summaries of actions approved before this migration).

ALTER TABLE pending_actions ADD COLUMN result_summary_json TEXT;
