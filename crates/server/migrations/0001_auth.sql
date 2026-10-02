-- Accounts and sessions for the web server (docs/HACKATHON.md §1 "Account management"). Lives
-- in its own schema (`auth` by default), owned by the server's login role; tenant roles get no
-- access to it. Instants are UTC RFC 3339 TEXT, as in the core schema.

CREATE TABLE users (
  id             TEXT PRIMARY KEY,          -- UUID v7
  username       TEXT NOT NULL UNIQUE,      -- normalized: lowercase
  password_hash  TEXT NOT NULL,             -- Argon2id PHC string
  schema_name    TEXT NOT NULL UNIQUE,      -- the user's tenant schema and role, u_<id hex>
  created_at     TEXT NOT NULL
);

CREATE TABLE sessions (
  token_hash  TEXT PRIMARY KEY,             -- hex SHA-256 of the cookie token; the token itself is never stored
  user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  created_at  TEXT NOT NULL,
  expires_at  TEXT NOT NULL
);
CREATE INDEX sessions_user_id ON sessions(user_id);
