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
| **Runs on a Google Cloud service** (App Engine, Compute Engine, GKE, Cloud Run, Cloud Run functions or Cloud TPU/GPU) | ❌ Missing. It's a local Windows Tauri app with no cloud component. |
| **Uses a Google AI service** (Gemini API, ADK, Vertex AI / Gemini Enterprise Agent Platform, …) | ⚠️ Nearly there. Gemini goes through Google's OpenAI-compatible endpoint, and the FAQ says using an AI Studio key directly counts. But the 2026-10-01 DECISIONS entry says it has never been tested against a real Gemini model. |
| **No project already well along before the event started** | ✅ The first commit is 2026-09-23, after registration opened on 2026-08-20. Building on the project during the hackathon is explicitly allowed. |
| **Deployed URL, plus a test account if login is required** | ❌ Missing. |
| **Architecture diagram, 3-minute YouTube demo, project description** | ❌ Not made yet. The Zenn article (category "Idea") is optional. |
| **Eligibility: lives in Japan, aged 18+** | The participant has to confirm this. |

## What to change

### 1. Deploy to Cloud Run (required, critical path)

The event asks for a public web version where possible. For desktop apps, features that only
work on the desktop just need to be documented, and the app is judged mainly from the demo
video. The cleanest route here:

- Add a small Rust HTTP server crate, e.g. `crates/server` with axum, as a third composition
  root next to `cli` and `desktop`. It wraps the existing `CoreApp`, so the core and plugin
  logic is reused unchanged.
- Point the React UI at HTTP calls instead of Tauri `invoke`. Since the frontend is already a
  thin view layer, this is mostly swapping the transport.
- Three parts assume a single local machine and need cloud versions:
  - **Secrets:** OS keyring → Secret Manager, or Vertex AI with the service account so there's
    no key at all.
  - **SQLite:** Cloud Run's disk is ephemeral. For a demo, one instance
    (`max-instances=1`) is acceptable if losing data is OK. Otherwise use Cloud SQL, which
    means replacing the sqlx SQLite queries with Postgres ones.
  - **Single user:** add a login and a demo test account.
- The desktop app can stay as the main product, with the web build as the judges' URL.

### 2. Make Gemini real

- Run a real chat turn that logs a meal against a Gemini 3 model, as the 2026-10-01 entry says
  is still to do.
- Make Gemini the default provider for this build.
- Optionally call it through Vertex AI, which the event recommends (Gemini Enterprise Agent
  Platform). That uses Cloud Run's service account, so there's no API key to manage.

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
  (chat or scheduler).
- **Human approval for risky actions:** past-day deletes and memory writes go through a
  confirmation step instead of only a flag the model sets.
- **Prompt-injection defense:** treat imported food tables and dish names as untrusted data.
- **Per-tool permissions and cost/token budgets**, built on the existing usage sink.
- **More autonomy:** the scheduler already generates the next-day plan. Proactive follow-ups
  (e.g. "you haven't logged lunch") would show the agent acting on its own. Show this in the
  video.

### 4. Submission materials

- An architecture diagram covering Cloud Run, Gemini, the plugin system and the DB.
- A 3-minute demo video (YouTube link).
- A project description covering target users, the problem and the solution.
- Consider switching the UI from Chinese to Japanese for the demo. The strings are all in
  `apps/desktop/ui/src/i18n/zh-CN.ts`, so adding a `ja-JP` file is cheap.
- The event page summary says the repository should stay unchanged until the final pitch on
  2026-12-01. Check the exact wording. If it applies, submit the `g-hackathon` branch and keep
  working elsewhere, e.g. on `phase1`.

## Suggested order

1. Server crate and Cloud Run deployment (everything else depends on having a URL).
2. Real Gemini verification, Gemini as the default provider.
3. Governance features: audit trail, approval flow, injection defense.
4. Japanese UI strings, architecture diagram, demo video, description.
