# Decisions

Deviations from `PHASE1_SPEC.md` and resolutions to ambiguities it left open, in the order they
were made. Per `CLAUDE.md`: "If something is ambiguous, pick the simplest option consistent with
the spec, note the decision here, and continue."

Open items awaiting a decision are tracked in `docs/BLOCKERS.md`, not here — an entry moves from
there to here once resolved.

## 2026-09-23 — M0: closed. `lifeagent-desktop` build/run verified after system packages installed

`docs/PHASE1_SPEC.md` §14 M0 "done when" requires `cargo build --workspace` and
`npm run tauri dev` to start, with clippy clean. Initially blocked in this Linux/WSL sandbox:
`pkg-config` itself was not installed, and the sandbox had no non-interactive `sudo` to install
it or the GTK/WebKit/D-Bus dev packages Tauri 2 needs to link on Linux.

Resolved by installing, on this machine:

```
sudo apt install -y pkg-config libwebkit2gtk-4.1-dev libjavascriptcoregtk-4.1-dev \
  libsoup-3.0-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev \
  libdbus-1-dev patchelf
```

After that, full verification passed:

- `cargo build --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo fmt --all -- --check`, `cargo test --workspace` — all clean, including
  `lifeagent-desktop`.
- `npm run tauri dev` (run from `apps/desktop`, see the layout fix below) opened the main
  window at the configured 1100×720 size with the WebKit webview rendering the React/Tailwind
  page and the `zh-CN.ts` string, confirmed by screenshot (`import -window <id>`, with
  `GDK_BACKEND=x11` forced — WSLg's default GTK/Wayland surfaces aren't visible to X11
  screenshot tools, so this override was needed for verification only; not something the app
  needs to set itself, and not something the desktop app's own code should force at runtime).

**Layout fix made during this verification:** the `ui/package.json` originally owned the
`tauri` npm script and `@tauri-apps/cli` devDependency, colocated with the Vite/React files.
Run from `apps/desktop/ui`, the Tauri CLI searches only *subfolders* of its cwd for
`tauri.conf.json` and cannot find `apps/desktop/src-tauri/tauri.conf.json`, which is a sibling,
not a child. This also didn't match `CLAUDE.md`'s own documented command
(`cd apps/desktop && npm install && npm run tauri dev`).

Fixed with an npm workspace: `apps/desktop/package.json` (workspace root, `"workspaces":
["ui"]`) now owns `@tauri-apps/cli` and the `tauri` script; `apps/desktop/ui/package.json` keeps
the frontend-only `dev`/`build`/`preview` scripts and dependencies. `tauri.conf.json`'s
`beforeDevCommand`/`beforeBuildCommand` use the object form (`{"script": "npm run dev", "cwd":
"../ui"}`) since those hooks run relative to `tauri.conf.json`'s own directory (`src-tauri`),
not the invocation cwd. `CLAUDE.md`'s documented command now works as written.

## 2026-09-23 — M1: closed. Foundations built without pulling `lifeagent-llm` types forward

`docs/PHASE1_SPEC.md` §14 M1 covers Clock, logical date, DB + core migrations, settings,
secrets, profile/goal services, the event store + bus, memory notes and the conversation store.
Two places in the spec's own text for these modules refer to a type that, per the milestone
order, does not exist yet:

- §7.2 lists a `models: ModelsConfig` setting, but `ModelsConfig` is defined in
  `lifeagent-llm` §6.3, which is M2.
- §7.10's conversation store speaks in terms of `lifeagent-llm::Message` variants
  (assistant/tool/...), same M2 dependency.

Resolved both without touching `lifeagent-llm` in M1, so the crate boundary and milestone order
stay intact:

- `settings.rs` implements `get_json<T>`/`set_json<T>` generically (`T: Serialize` /
  `DeserializeOwned`) over the existing `key -> value_json` table, plus concrete accessors for
  the four settings whose type is already known (`plan_time`, `day_boundary`,
  `notifications_enabled`, `autostart`, `onboarding_complete`). `models`/`set_models` will be a
  thin `get_json::<ModelsConfig>("models")` / `set_json(...)` wrapper added in M2 — no change to
  `settings.rs` needed then.
- `conversation.rs` defines its own `MessageRole`/`StoredMessage`/`NewMessage`, shaped directly
  after the `messages` table's columns (role/content/tool_calls_json/tool_call_id/tool_name)
  rather than `lifeagent-llm::Message`. This is a reasonable fit regardless of the type-ordering
  issue: the table is already row-shaped, not enum-shaped, so a `StoredMessage` matches it more
  directly than the wire-format `Message` would. Translating `StoredMessage` into
  `lifeagent-llm::Message` for a `ChatRequest` becomes the orchestrator's job in M3, which is
  also the first place in `core` that needs to depend on `lifeagent-llm`'s types for real.

Also clarified: `CoreApp`/`CoreAppBuilder` (§7.1) are **not** part of M1 despite being the first
thing §7 describes — §14 explicitly assigns "`CoreApp` builder and `send_message`" to M3, since
building it needs the plugin trait, tool registry and orchestrator loop that don't exist until
then. M1 delivers the standalone service modules (`db`, `clock`, `time`, `settings`, `secrets`,
`profile`, `goals`, `events`, `memory`, `conversation`) that M3's `CoreAppBuilder` will wire
together; each is independently constructed (`SqlitePool` + `Arc<dyn Clock>`, mostly) and
independently tested, which is also why M1's "done when" in §14 names per-module test coverage
rather than an integration test through `CoreApp`.

Two SQL ordering bugs surfaced by the tests themselves, fixed rather than filed: `memory.rs`'s
`list()` and `events.rs`'s `list()`/`latest()` originally ordered only by a stored timestamp
column; under a `FixedClock` (and in principle under real time, for two events logged within
the same second) two rows can share that timestamp, making "newest/oldest first" order
unspecified. Added `rowid` as a secondary sort key (SQLite's implicit, strictly-increasing
insertion-order column) everywhere this could bite.

## 2026-09-24 — M2: closed. LLM layer built; `effort` added to `ChatRequest`

`docs/PHASE1_SPEC.md` §6 gives `ChatRequest` six fields (`model`, `messages`, `tools`,
`temperature`, `max_tokens`, `response_format`) and separately gives `RoleBinding` an
`effort: Option<Effort>` used "Anthropic only" (§6.3's table: parser=low, planner=high,
conversation=medium, mirroring `temperature`'s per-role defaults for the OpenAI-compatible
kind). But `ChatModel::complete` takes only `ChatRequest`, and the router dispatches to
whichever provider by calling that one method — so there is no path left for a resolved
`effort` value to reach the Anthropic provider once `RoleRequest` (built by the caller) is
merged with the binding. This is the same class of defect as `docs/BLOCKERS.md`'s B1–B4
(expression-level gap, not a design problem), just not filed there since it blocks M2 itself
rather than M4/M5.

Resolved by adding `pub effort: Option<Effort>` to `ChatRequest` (`crates/llm/src/types.rs`),
symmetric with `temperature`: each provider reads the field it needs and ignores the other,
exactly as §6.2.2 already says of `temperature` ("ignored for `ProviderKind::Anthropic`").
`RoleRequest` (§6.3: "`ChatRequest` without `model`/`temperature`") is defined to also exclude
`effort`, for the same reason it excludes `temperature`: both are router-resolved from the
binding, not caller-supplied. The router computes `(temperature, effort)` once per call from
`ProviderKind` and either the binding's explicit value or the per-role default table.

Also: `Effort` itself is defined in `types.rs` rather than `router.rs` (where §6.3's code block
places it), purely because `ChatRequest` needed it and `types.rs` (§6.1) is a lower module than
`router.rs` (§6.3); `router.rs` uses `crate::types::Effort`. No shape change, just which file
owns the `enum`.

Two smaller gaps, resolved without a decision needed (simplest compliant choice per
`CLAUDE.md`):

- `LlmError::InvalidResponse` and `LlmError::Config` carry a `String` (spec names only the
  variants, not their fields, for these two — `Status`'s and `RateLimited`'s shapes were
  already spelled out).
- `KeyProvider::get_key` and `UsageSink::record`/`UsageEvent` aren't given field-level shapes
  in §6.3 beyond "a `KeyProvider`" / "a `UsageSink` trait ... writes `llm_usage`". `UsageEvent`
  mirrors the `llm_usage` table columns (§5) minus `id`/`created_at`, which the sink
  implementation (in `lifeagent-core`, M3/M5 per `usage.rs`'s existing stub) assigns.

Provider mapping (`build_request_body`/`parse_response_body` in both `provider/*.rs`) is kept
as pure functions separate from the HTTP-calling `ChatModel::complete`, so the "provider
mapping test suites... with fixtures" `§14` done-when criterion is testable without a live or
mocked HTTP server. Retry/backoff policy (timeout, retryable-status classification, `Retry-After`
parsing, body-excerpt truncation) is similarly split into pure functions in `provider/mod.rs`
and tested directly; the HTTP-calling glue around them is exercised only by real network use
(out of scope for offline tests, per `CLAUDE.md`).

Verification: `cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings` and
`cargo fmt --all -- --check` all pass for `lifeagent-llm`, `lifeagent-core`,
`lifeagent-plugin-diet` and `lifeagent-cli` (122 tests total, including the new provider
mapping fixture tests, router role-resolution tests, and `complete_json` repair-loop tests).
`lifeagent-desktop` was not rebuilt in this session: the sandbox's apt mirror returned 404 for
the WebKit/GTK dev packages M0's `DECISIONS.md` entry lists (mirror snapshot mismatch, not a
code issue); M2 doesn't touch `apps/desktop` or `lifeagent-core`'s M3+ modules, so this doesn't
gate M2's own done-when criteria.
