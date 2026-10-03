-- Preferences replace memory notes (docs/DECISIONS.md, "Preferences replace memory notes").
-- Each has a kind ('dislike' | 'general') and an optional range of logical dates, both ends
-- inclusive. Existing memory notes become open-ended 'general' preferences, and the profile's
-- dislikes become open-ended 'dislike' preferences.

CREATE TABLE preferences (
  id           TEXT PRIMARY KEY,
  seq          BIGINT GENERATED ALWAYS AS IDENTITY,
  kind         TEXT NOT NULL,          -- 'dislike' | 'general'
  text         TEXT NOT NULL,
  valid_from   TEXT,                   -- logical date (YYYY-MM-DD), inclusive; NULL = open
  valid_until  TEXT,                   -- logical date (YYYY-MM-DD), inclusive; NULL = open
  created_at   TEXT NOT NULL,
  deleted_at   TEXT
);

-- Migrated rows keep their ids; the dislikes get random v4 ids, since Postgres 17 can't make
-- v7 ones (ordering uses created_at and seq, never the id).
INSERT INTO preferences (id, kind, text, created_at, deleted_at)
SELECT id, 'general', note, created_at, deleted_at FROM memory_notes ORDER BY seq;

INSERT INTO preferences (id, kind, text, created_at)
SELECT gen_random_uuid()::text, 'dislike', trim(d.value), p.updated_at
FROM profile p,
     jsonb_array_elements_text(COALESCE(p.data_json::jsonb -> 'dislikes', '[]'::jsonb))
       WITH ORDINALITY AS d(value, n)
WHERE trim(d.value) <> ''
ORDER BY d.n;

UPDATE profile SET data_json = (data_json::jsonb - 'dislikes')::text;

DROP TABLE memory_notes;
