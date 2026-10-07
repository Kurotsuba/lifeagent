# Hackathon readiness: 第5回 Agentic AI Hackathon with Google Cloud

Event page: https://zenn.dev/hackathons/google-cloud-japan-ai-hackathon-vol5
Assessed: 2026-10-02, on branch `g-hackathon` (from `phase1` at `47df6bc`).

## Key dates

| Date | What |
|---|---|
| 2026-08-20 | Registration opened (event start) |
| 2026-10-15 23:59 | Registration and submission deadline |
| 2026-10-28 | Finalists announced |
| 2026-12-01 | Final pitch and awards |

## Theme and judging

Theme: 「"作れる"から"使える"。エージェントの、新しいステージへ。」 This round is about
governance and security: agents you can trust in real use, plus defenses against prompt
injection.

Judging criteria:

1. **課題の新規性と解決策の有効性**: does it find a problem many people have but nobody has
   solved, and does the solution address the core of that problem?
2. **自律性・エージェントらしさ**: does it go beyond simple automation and act on its own
   judgment? Is that autonomy well managed and controlled, with attention to observability and
   security?
3. **実装品質と拡張性**: implementation quality, use of the right tools, extensibility,
   operability and cost-effectiveness.

## Requirements check

| Requirement | Status |
|---|---|
| **Runs on a Google Cloud service** (App Engine, Compute Engine, GKE, Cloud Run, Cloud Run functions or Cloud TPU/GPU) | ✅ Cloud Run (`lifeagent`, asia-northeast1) with Cloud SQL for PostgreSQL, Secret Manager and Cloud Scheduler, deployed 2026-10-02. |
| **Uses a Google AI service** (Gemini API, ADK, Vertex AI / Gemini Enterprise Agent Platform, …) | ✅ Gemini on Agent Platform, authenticated with Google Cloud access tokens (§2). A real chat turn through the web build worked on 2026-10-02. |
| **No project already well along before the event started** | ✅ The first commit is 2026-09-23, after registration opened on 2026-08-20. Building on the project during the hackathon is explicitly allowed. |
| **Deployed URL, plus a test account if login is required** | ⚠️ URL: https://lifeagent-519106969791.asia-northeast1.run.app. A demo account for the judges still has to be created and listed in the submission. |
| **Architecture diagram, 3-minute YouTube demo, project description** | ❌ Not made yet. The Zenn article (category "Idea") is optional. |
| **Eligibility: lives in Japan, aged 18+** | The participant has to confirm this. |

## What to change

### 1. Deploy to Cloud Run (required, critical path)

The event asks for a public web version where possible. For desktop apps, features that only
work on the desktop just need to be documented, and the app is judged mainly from the demo
video. The desktop app can stay as the main product, with the web build as the judges' URL.

Update 2026-10-03: the desktop app is not needed for the hackathon and was removed from this
branch; the UI now lives in `apps/web` (`docs/DECISIONS.md`, "Desktop app removed").

#### Decisions (2026-10-02)

- The Rust side becomes an HTTP backend: a new `crates/server` crate (axum) as a third
  composition root next to `cli` and `desktop`. It wraps the existing `CoreApp`, so the core
  and plugin logic is reused.
- The desktop UI becomes a plain React web frontend: swap Tauri `invoke` for `fetch`. The
  frontend is already a thin view layer, so this is mostly the transport.
- All SQLite storage moves to Postgres.
- Everything else stored locally also moves to Postgres, except logs.
- The hackathon build calls Gemini on Agent Platform as the Cloud Run service account, with no
  API key (see §2). So the keyring-backed API key storage does not need converting.

#### What is stored locally today

Besides SQLite and logs, only two things:

| What | Where | Plan |
|---|---|---|
| API keys | OS credential store via `KeyringSecretStore` (`crates/core/src/secrets.rs`), service `LifeAgent`, account `provider:{id}` | Not converted. Gemini uses the service account's access token (§2); there is no key. |
| Daily DB backups | `data_dir/backups/lifeagent-YYYY-MM-DD.db`, written by `backup_if_due` (`crates/core/src/scheduler.rs`) with SQLite's `VACUUM INTO` | Removed. Cloud SQL automated backups replace it. |

Already in SQLite, so they move with the rest: settings (including the model/provider config
and the `autostart` flag), profile, goals, events, plans, memory notes, messages, LLM usage,
and the diet plugin's own tables (foods, sources).

Desktop-only, with no data to move: Windows autostart registration, notifications, tray icon,
single-instance lock, quick-entry window. The CLI's `import`/`export` read and write
user-named files; the web version uses upload and download. The React UI keeps no state of its
own (no `localStorage`, IndexedDB or Tauri store).

#### GCP services

| AWS | GCP | Fit |
|---|---|---|
| Lambda | Cloud Run functions | Poor: no Rust runtime. |
| Lambda container images / Fargate | **Cloud Run** (service) | **Use.** Container with the axum server; scales to zero, billed per request. Qualifies as the required compute service. |
| DynamoDB | Firestore | Not used: it would mean rewriting all the SQL. |
| RDS Postgres | **Cloud SQL for PostgreSQL** | **Use.** The code already uses SQL through sqlx. |
| EventBridge Scheduler | **Cloud Scheduler** | **Use**, to drive the scheduler (see below). |
| Secrets Manager | **Secret Manager** | **Use**, for the DB password. |

#### Architecture

```
Browser --HTTPS--> Cloud Run service (one container)
                     |- axum: /api/* JSON endpoints
                     |- axum: serves the built React files (static)
                     |- Gemini on Agent Platform (service account token, no key)
                     '- Cloud SQL Postgres (built-in Cloud Run <-> Cloud SQL connection)
Cloud Scheduler --> POST /api/internal/tick (replaces the in-process scheduler loop)
```

- **One container for frontend and API:** one URL, no CORS, one deploy.
- **Scheduler:** `crates/core/src/scheduler.rs` runs a loop inside the process. Cloud Run only
  gives an instance CPU while it handles a request and scales to zero, so the next-day plan
  and proactive follow-ups would silently stop. Fix: Cloud Scheduler calls
  `/api/internal/tick` every few minutes with a shared secret (`X-LifeAgent-Tick-Token`,
  from `LIFEAGENT_TICK_TOKEN`), and the handler runs one tick of the existing logic for every
  user (implemented 2026-10-02; a shared secret rather than OIDC, see `docs/DECISIONS.md`). This also fits the theme, because every autonomous action becomes an
  auditable request. Fallback: `--min-instances=1 --no-cpu-throttling` and keep the loop, at
  the cost of an always-on instance.
- **One instance (`--max-instances=1`) for the demo.** Each instance caches its own
  `CoreApp` per user, and a user's live event stream (`/api/events`) only sees changes made
  through the same instance. Login rate limits are per instance too.
- **Time zone:** core computes logical dates and `plan_time` in the server's local time zone
  (chrono `Local`), and Cloud Run runs in UTC. Set `TZ=Asia/Tokyo` on the service. Per-user
  time zones are not supported.
- **Gemini:** called as the Cloud Run service account (`roles/aiplatform.user`), with tokens
  from the metadata server. No key anywhere. See §2.
- **DB password:** in Secret Manager, exposed with `--set-secrets DB_PASSWORD=...`. Never in
  the image or the repo.
- **Logs:** to stdout as Cloud Logging structured JSON (`LIFEAGENT_LOG_FORMAT=json`, set in
  `deploy/env.yaml`; done 2026-10-07, `docs/DECISIONS.md` "Structured logs for Cloud Logging").
  The rolling log file is not used on the server.
- **Accounts:** register, login and logout, with each user's data in its own Postgres schema.
  See "Account management" below.

#### Account management

Requirement: users can register, log in and log out. Each user's data lives in its own
Postgres schema, so data access is separated per user.

**Layout**

```
schema auth                 shared, owned by the app
  users      (id uuid v7, username unique, password_hash, schema_name, created_at)
  sessions   (token_hash, user_id, created_at, expires_at)

schema u_<user id hex>      one per user, created at registration
  settings, profile, goals, events, plans, preferences, messages, llm_usage,
  plugin_migrations, diet plugin tables, _sqlx_migrations
```

The per-user schema holds exactly today's single-user tables, so core and plugin queries stay
single-user. They never mention a user id and find their tables through `search_path`.

**Isolation** (implemented in `crates/core/src/db.rs`, 2026-10-02)

A schema alone only separates names: one DB role can still read every schema. To make the
separation real:

- Each user schema is owned by its own `LOGIN` role `u_<user id hex>`, with no grants to
  anyone else, and every connection in that user's pool logs in as it (`DbConfig::tenant`).
  The user role is a member of no other role, so even a connection running arbitrary SQL can't
  `SET ROLE` or `SET SESSION AUTHORIZATION` into the app's role or another user: Postgres
  refuses ("permission denied"), as it refuses any query naming another user's schema. This is
  worth showing for the security theme.
- The app's own login role creates every user role (so it holds `ADMIN` on them) but keeps no
  `SET` membership, so it has no access to user data while acting as itself. It grants itself
  `SET` only inside the provisioning and account-deletion transactions.
- A user role's password is never stored: it is HMAC-SHA256 of the role name keyed by the
  app's database URL (which carries the app's password), and Postgres receives only a SCRAM
  verifier. No extra secret to deploy. `db::open` re-applies it on every open, which also
  upgrades users created before this change (2026-10-03) and follows a rotated `db-password`.
- Schema and role names come only from the generated user id, never from user input, and
  `db::open` rejects anything but `[a-z_][a-z0-9_]*`. Identifiers can't be bind parameters in
  DDL, so this is what keeps the `format!` safe.
- Tests run the same way: every test is its own tenant, opened by a non-superuser
  `CREATEROLE` login role like Cloud SQL's built-in users.

**Connections**

One `CoreApp` per logged-in user, built from `DbConfig::tenant(url, "u_<id>")`: a small pool
(max 2 connections, 60 s idle timeout), cached in the server and evicted when idle. Cloud SQL's
smallest tier allows only about 25 connections, so this fits a demo but not many concurrent
users. The scaling path is one shared pool with `SET LOCAL ROLE` / `SET LOCAL search_path` at
the start of each transaction, which means passing a user-scoped transaction through core
instead of a pool.

**Registration, in one transaction** (Postgres DDL is transactional, so a failure leaves
nothing half-created):

1. Validate the username and password (minimum length), insert into `auth.users`.
2. Build the user's `CoreApp` from `DbConfig::tenant(url, "u_<id>")`. `db::open` creates the
   role (`LOGIN`, derived password), creates the schema owned by it, and runs core and plugin
   migrations logged in as it. Idempotent, and serialized with an advisory lock.
3. Create a session and log the user in.

`db::open` commits the role and schema in its own transaction, so a registration that fails
after step 2 leaves an empty tenant behind. Clean it up with `db::destroy`, which also serves
account deletion (drops the schema with everything in it, then the role).

Migrations also run for every existing user schema at server start, so a deploy with a new
migration upgrades everyone.

**Sessions**

- Passwords hashed with Argon2id (`argon2` crate).
- Login creates a random 256-bit token, stored in `auth.sessions` as a SHA-256 hash, sent as
  an `HttpOnly; Secure; SameSite=Lax` cookie with an expiry (e.g. 30 days).
- Logout deletes the session row and clears the cookie.
- An axum extractor resolves the cookie to a user and their `CoreApp`. Every `/api/*` route
  except register and login requires it.
- Same generic error for a wrong username or wrong password. Basic per-IP rate limit on login
  and register.
- A demo test account is seeded for the judges and listed in the submission.

**Scheduler with many users**

The Cloud Scheduler tick iterates over `auth.users` and runs one scheduler tick per user,
each with a connection bound to that user's schema.

**Cost control**

All users share one Gemini bill, so add a per-user daily token budget built on `llm_usage`
(see §2). Open registration could otherwise run up the bill. Consider
closing registration after judging.

#### Cost

Cloud Run's free tier covers hackathon traffic. Cloud SQL is the only always-on part: an
instance runs constantly even at the smallest tier (shared-core `db-f1-micro`, roughly
$10/month). New GCP accounts get trial credit. Stop or delete the instance after judging.

#### Steps

1. **Port sqlx from SQLite to Postgres** (done 2026-10-02). `?` placeholders to `$1`,
   SQLite-only SQL (`VACUUM INTO`, `json_each`, `strftime`), migrations. Tests run against a
   local Postgres, one schema per test (see the 2026-10-02 entries in `docs/DECISIONS.md`).
2. **Per-user schemas in core** (done 2026-10-02): `DbConfig::tenant`, `db::open` provisioning
   the role and schema, `db::destroy`, isolation tests.
3. **Add `crates/server`** (done 2026-10-02): axum, `POST /api/{command}` mirroring the Tauri
   commands through the shared `lifeagent-api` crate, the `auth` schema,
   register/login/logout/delete-account, session extractor, per-user `CoreApp` cache, SSE
   events, the tick endpoint. Run locally with
   `LIFEAGENT_MOCK_LLM=1 cargo run -p lifeagent-server` (see README).
4. **Switch the React UI** (done 2026-10-02): one UI for both shells. `src/api/transport.ts`
   uses Tauri `invoke` on the desktop and `fetch` to `/api/{command}` in a browser;
   `/api/events` (SSE) replaces Tauri events. Web only: login/register screen, username and
   logout in the sidebar, 账号 settings section (account deletion); the model step,
   model settings and the notification/autostart switches are hidden. Verified end to end in
   headless Chromium against the real server.
5. **Deploy** (2026-10-02): `Dockerfile` (Vite build, Rust release build, Debian slim runtime
   serving both) and `deploy/deploy.sh` (`build`: Cloud Build on `e2-highcpu-8`; `deploy`:
   Cloud Run with `lifeagent-run`, the Cloud SQL socket, `db-password`/`tick-token` secrets,
   `deploy/env.yaml`, one instance; `schedule`: the Cloud Scheduler job every 5 minutes).
   Redeploy after a change with `deploy/deploy.sh` from a clean checkout.
6. **CI** (2026-10-03): `.github/workflows/ci.yml` runs on every push to `g-hackathon` only.
   `verify` runs fmt, clippy and the tests against a `postgres:17` service container, then
   `npm ci && npm run build` for the UI. If it passes, `deploy` runs `deploy/deploy.sh build`
   and `deploy` (not `schedule`). It authenticates through Workload Identity Federation as
   `lifeagent-ci`, with no key; `deploy/setup-ci.sh` creates both once. The provider only
   accepts pushes to `g-hackathon` of `Kurotsuba/lifeagent`. `deploy/deploy.sh` stays the way
   to deploy by hand.

#### Google Cloud console checklist

Menu names may differ slightly; the console search bar finds each page by name.

**Do now**

1. **Project and billing.**
   - Create a project (e.g. `lifeagent-hackathon`) and note its **Project ID**, which goes
     into the Gemini base URL.
   - Link a billing account (Agent Platform, Cloud Run and Cloud SQL all need it).
   - **Billing → Budgets & alerts:** a budget for this project (e.g. ¥3,000) with alerts at
     50%, 90% and 100%.
2. **Enable APIs** (**APIs & Services → Library**): Vertex AI API (`aiplatform.googleapis.com`,
   still the API name for Agent Platform), Cloud Run Admin, Cloud SQL Admin, Secret Manager,
   Cloud Scheduler, Artifact Registry, and Cloud Build if images are built in the cloud.
3. **Check Gemini.**
   - Send a test prompt to a Gemini model in the Agent Platform console.
   - Then run the `gcloud auth application-default login` + curl test from §2, which is the
     call the app makes.
   - No API key is needed. An `AQ.` key, if created, can be deleted under
     **APIs & Services → Credentials**.
4. **Service accounts** (**IAM & Admin → Service Accounts**). Create no JSON keys for either;
   Cloud Run supplies credentials.
   - `lifeagent-run`, the app's runtime identity: **Vertex AI User**
     (`roles/aiplatform.user`), **Cloud SQL Client**, **Secret Manager Secret Accessor**.
   - No separate scheduler account: Cloud Scheduler authenticates to `/api/internal/tick`
     with a shared secret header (step 9), not with a Google identity.
5. **Artifact Registry:** a Docker repository `lifeagent` in `asia-northeast1`.
6. **Gemini quota (optional):** **IAM & Admin → Quotas & system limits**, lower the Gemini
   quotas for the models in use. A hard limit on top of the budget alert and the per-user
   limits.

**Do later, once the Postgres port passes locally** (Cloud SQL costs money just by existing)

7. **Cloud SQL** (**SQL → Create instance → PostgreSQL**).
   - Settings: Enterprise edition, smallest shared-core machine (`db-f1-micro`) or the Sandbox
     preset, PostgreSQL 17, `asia-northeast1`, single zone, automated backups on.
   - No authorized networks: Cloud Run uses its built-in Cloud SQL connection; local access
     goes through the Cloud SQL Auth Proxy.
   - Then create database `lifeagent` and user `lifeagent_app` with a strong password.
     Users created this way are in `cloudsqlsuperuser` and have `CREATEROLE`, which
     registration needs to create each user's own login role.
8. **Secret Manager:** secret `db-password` with the `lifeagent_app` password, and
   `tick-token` with a long random string (e.g. `openssl rand -hex 32`) for the scheduler.
9. **After the first deploy.**
   - **Cloud Run:** confirm the service runs as `lifeagent-run`, has the Cloud SQL connection,
     maps the secrets to env vars (`LIFEAGENT_DATABASE_URL` built with `db-password`,
     `LIFEAGENT_TICK_TOKEN` from `tick-token`), sets `TZ=Asia/Tokyo`, allows unauthenticated
     access (the app has its own login), and has max instances 1.
   - **Cloud Scheduler:** POST to `<service URL>/api/internal/tick` every 5 minutes, auth
     **None**, with header `X-LifeAgent-Tick-Token: <tick-token value>`. The server checks
     it, since the service itself is public.

### 2. Make Gemini real

#### Decision (2026-10-02): Gemini on Agent Platform, service-account auth

Only the backend calls Gemini, through Gemini Enterprise Agent Platform (formerly Vertex AI),
authenticated as the Cloud Run service account. This replaces the earlier "fixed Gemini key in
Secret Manager" plan.

Why:

- **No key to leak or rotate.** Access is one IAM role, revocable in one place, and Cloud Audit
  Logs record which identity called the model. Google recommends API keys for testing and
  application default credentials for production.
- **Data terms.** Users' meals and health goals are personal data. Agent Platform runs under
  Google Cloud's data terms (no training on customer data, data-residency controls). The AI
  Studio free tier may use prompts to improve Google's products.
- **Operations.** Cloud Monitoring metrics, per-project quotas, IAM and billing all sit with the
  rest of the stack.
- **The event** recommends Agent Platform.

An express-mode `AQ.` API key does not work here: Agent Platform's OpenAI-compatible endpoint
accepts only Google Cloud access tokens ("Only Google Cloud Auth is supported using the OpenAI
library in Gemini Enterprise Agent Platform").

#### Connection settings

| Setting | Value |
|---|---|
| Provider kind | OpenAI-compatible |
| Base URL (服务地址) | `https://aiplatform.googleapis.com/v1/projects/<PROJECT_ID>/locations/global/endpoints/openapi` |
| Model names | `google/` prefix, e.g. `google/gemini-3.5-flash`; pin exact versions for judging |
| Auth | `"auth": "google_cloud"` on the provider: `Authorization: Bearer <access token>`, fetched and refreshed by the server (tokens last 1 hour) |
| IAM | Runtime service account gets `roles/aiplatform.user`; enable `aiplatform.googleapis.com` |

Use the `global` location: it routes around regional capacity shortages.

Local test, before any code change:

```bash
gcloud services enable aiplatform.googleapis.com
gcloud auth application-default login
curl -s "https://aiplatform.googleapis.com/v1/projects/$PROJECT/locations/global/endpoints/openapi/chat/completions" \
  -H "Authorization: Bearer $(gcloud auth application-default print-access-token)" \
  -H "Content-Type: application/json" \
  -d '{"model":"google/gemini-3.5-flash","messages":[{"role":"user","content":"Say hi"}]}'
```

#### Code change: refreshing token source (done 2026-10-02)

A provider with `"auth": "google_cloud"` sends a Google Cloud access token instead of a key
(`crates/llm/src/google_auth.rs`), cached and refreshed 5 minutes before it expires. Where the
token comes from, the standard Application Default Credentials order:

- `GOOGLE_APPLICATION_CREDENTIALS`, else gcloud's ADC file (`gcloud auth
  application-default login`): the developer's own login, refreshed at Google's OAuth endpoint.
- Otherwise the metadata server: on Cloud Run, the service account's token. No key anywhere.

`LIFEAGENT_MODELS` for the server:

```json
{
  "providers": [{
    "id": "vertex", "kind": "open_ai_compatible", "auth": "google_cloud",
    "base_url": "https://aiplatform.googleapis.com/v1/projects/<PROJECT_ID>/locations/global/endpoints/openapi"
  }],
  "parser":       { "provider_id": "vertex", "model": "google/gemini-3.5-flash-lite" },
  "planner":      { "provider_id": "vertex", "model": "google/gemini-3.1-pro-preview" },
  "conversation": { "provider_id": "vertex", "model": "google/gemini-3.8-flash" }
}
```

Use pinned model versions for judging. Gemini 3 models (`google/gemini-3…`) get no temperature,
as with AI Studio names. Token fetching was checked against Google's real endpoint with this
machine's ADC (`cargo test -p lifeagent-llm -- --ignored live_`); the regular tests run
offline against a local fake server.

#### Model change (2026-10-02): Gemini 2.5 → Gemini 3

The first Cloud Run deploy used Gemini 2.5 Flash (parser, conversation) and 2.5 Pro (planner).
`deploy/env.yaml` now binds:

| Role | Model | Why |
|---|---|---|
| Parser | `google/gemini-3.5-flash-lite` | Frequent, simple structured extraction: the cheapest model |
| Conversation | `google/gemini-3.8-flash` | Fast tool use |
| Planner | `google/gemini-3.1-pro-preview` | Hardest reasoning, about one call per user per day |

- The IDs were checked against Agent Platform's publisher-model metadata first. 3.8 Flash and
  3.5 Flash-Lite are GA; 3.1 Pro exists only as `-preview` (Public Preview), so it can change or
  be retired on short notice. If it disappears before judging, fall back to
  `google/gemini-2.5-pro` for the planner.
- All three are Gemini 3, so the provider sends no temperature.
- Deployed as revision `lifeagent-00003` on the unchanged image (`TAG=12b98fe deploy/deploy.sh
  deploy`; only the environment changed). Checked on production with a temporary account: a
  meal message was parsed and logged (2 items, 297 kcal, 21 s including food estimation for
  unseen foods) and a manual plan was generated by the 3.1 Pro preview (46 s).
- Details in `docs/DECISIONS.md` ("Gemini 3 models on Cloud Run").

#### Serving users safely

- **Backend only.** The browser talks to our API, never to Gemini.
- **Per-user limits.** Daily token budget from `llm_usage` plus per-user request rate limits.
  Lower the project's Gemini quota and set a billing budget alert so a runaway bug can't burn
  money.
- **Cap output.** `max_tokens` on every call.
- **Cheapest model per role.** Flash for parsing and conversation, Pro only for planning (the
  role-based router already supports this).
- **Failures.** Retry `429`/`5xx` with backoff and jitter (`send_with_retries`), then show a
  friendly "busy, try again".
- **Model output is untrusted.** Tool arguments are validated, numbers come from Rust, imported
  and user text is data not instructions, destructive actions need approval, and per-user DB
  roles stop a prompt-injected tool call from reaching other users' data.
- **Privacy.** No prompts or responses in logs at INFO; log metadata (user, model, tokens,
  latency, tool calls). Send the model only what the turn needs. Say in the UI that responses
  come from Gemini, and offer account deletion (`DROP SCHEMA ... CASCADE`).
- **Observability.** Tokens, latency, errors and cost per user and role, from `llm_usage` and
  structured logs in Cloud Logging.

#### Status

- Done 2026-10-02: a real chat turn through the web build (local server, browser UI) against
  Gemini on Agent Platform, authenticated with this machine's ADC, worked (manual test).
- Gemini on Agent Platform is the web build's provider through `LIFEAGENT_MODELS` (the desktop
  keeps its own model settings).
- Done 2026-10-02 on Cloud Run with the service account: Gemini 3.8 Flash (conversation),
  3.5 Flash-Lite (parser) and 3.1 Pro preview (planner), each exercised on production.

### 3. Lean into the theme

Judging scores autonomy together with control, observability and security. The project already
has a good base:

- tool arguments are validated
- the LLM is never allowed to produce final nutrition numbers
- the plan validator
- soft deletes
- the `past_day` delete guard
- usage tracking
- tracing

To make that visible:

- **Audit trail UI:** show every tool call, with its arguments, result and who triggered it
  (chat or scheduler). Done 2026-10-07: `docs/AUDIT_LOG_SPEC.md` (milestones AL0–AL4), the Activity tab.
- **Human approval for risky actions:** past-day deletes and preference/allergy writes go through a
  confirmation step instead of only a flag the model sets. Done 2026-10-05 (`docs/DECISIONS.md`,
  "Approval for sensitive tool calls").
- **Prompt-injection defense:** treat imported food tables and dish names as untrusted data.
  Done 2026-10-05 for the chat and planner prompts (`docs/DECISIONS.md`, "Untrusted data in
  prompts").
- **Per-tool permissions and cost/token budgets**, built on the existing usage sink. Token
  budget and per-user request limit done 2026-10-05 (`docs/DECISIONS.md`, "Token budget and
  request limit per user"); the budget is not switched on in `deploy/env.yaml` yet.
- **More autonomy:** the scheduler already generates the next-day plan. Proactive follow-ups
  (e.g. "you haven't logged lunch") would show the agent acting on its own. Show this in the
  video.

### 4. Submission materials

- An architecture diagram covering Cloud Run, Gemini, the plugin system and the DB.
- A 3-minute demo video (YouTube link).
- A project description covering target users, the problem and the solution.
- Consider switching the UI from Chinese to Japanese for the demo. The strings are all in
  `apps/web/src/i18n/zh-CN.ts`, so adding a `ja-JP` file is cheap.
- The event page summary says the repository should stay unchanged until the final pitch on
  2026-12-01. Check the exact wording. If it applies, submit the `g-hackathon` branch and keep
  working elsewhere, e.g. on `phase1`.

## Suggested order

1. Postgres port, server crate, web frontend and Cloud Run deployment (see §1 steps;
   everything else depends on having a URL).
2. Gemini on Agent Platform: refreshing token source, real verification, default provider.
3. Governance features: audit trail, approval flow, injection defense.
4. Japanese UI strings, architecture diagram, demo video, description.
