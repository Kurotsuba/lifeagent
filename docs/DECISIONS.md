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
