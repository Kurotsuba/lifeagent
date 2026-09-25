# LifeAgent — M7 Scope: Desktop

Scoping notes for Phase 1 milestone M7 (`docs/PHASE1_SPEC.md` §14/§10). Breaks the milestone
into six ordered sub-steps with their own "done when" criteria, so work can be checked off
incrementally instead of as one large undifferentiated task.

## Overview

M7 is "Desktop" — wiring the Rust core (`lifeagent-core` + `lifeagent-plugin-diet`, done
through M6) into the actual Tauri app. Per `docs/PHASE1_SPEC.md` §14: commands, events,
onboarding, Today, Chat, Settings, tray, close-to-tray, notifications, single instance,
autostart, quick-entry window.

It adds no new business logic — everything routes through the existing `CoreApp` surface, per
CLAUDE.md's "no business logic in the frontend" rule. The milestone is done when the full §15
acceptance scenario passes manually (not test-suite-verified, same as M6's real-API-key check).

## Cross-cutting infrastructure

Built once, used by every sub-step below.

1. **Done.** `CommandError` + mapping from `CoreError` (`src-tauri/src/error.rs`).
   `CoreError::NotConfigured` (new variant, added so this case doesn't need string-matching on
   `Validation`'s message) → `not_configured`; `Validation` → `invalid_input` (a generic zh-CN
   message for now — per-rule messages are M8 work, see `docs/DECISIONS.md`); `NotFound` →
   `not_found`; `NothingToPlan` → `invalid_input`; `LlmError::Auth` → `auth` (the variant already
   existed, so no HTTP-status sniffing was needed); `LlmError::Config` → `not_configured`;
   everything else → `internal`/`llm`, logged at `error!`/`debug!`. See `docs/DECISIONS.md`'s
   2026-09-25 M7.1-step-1 entry for the full reasoning.
2. **Done.** Composition root (`lib.rs`, new `paths.rs` mirroring the CLI's). Both
   `CoreAppBuilder::build()` and `start_scheduler()` run inside one `tauri::async_runtime::block_on`
   call in `setup()` (calling `start_scheduler()` after `block_on` returns panics — no Tokio
   runtime context on `setup()`'s own thread — see `docs/DECISIONS.md`'s step-2 entry), then
   `app.manage(core_app)`. Smoke-testing this against a real data dir also surfaced and fixed a
   second bug: a stored `models` config that fails to build (e.g. keyring session expired) now
   falls back to "not configured" instead of crashing `build()` outright.
3. **Done.** `AppEvent` → webview forwarding (`events.rs`), spawned in `setup()` after
   `core_app` is built. Payload shape per event (not specified by the spec, so picked directly
   from each variant's fields): `data-changed` → `{ event_type, logical_date }`; `plan-ready` →
   `{ date, version, trigger }`; `plan-failed` → `{ date, error }`. The event→payload mapping is
   a separate pure function with its own unit tests, since the surrounding loop needs a live
   `AppHandle` this sandbox can't exercise against a real webview. Notification triggering on
   `PlanReady`/`PlanFailed` is its own step (#4, `notify.rs`), not folded into this one.
4. **Done, with one gap.** Notifications (`notify.rs`): on `PlanReady` with `trigger ==
   Schedule`, and on any `PlanFailed`, show a `tauri-plugin-notification` toast with the zh-CN
   titles from §10.1 (body = first sentence of the plan summary, or a fixed generic zh-CN
   message for failures — the underlying `CoreError` text stays in the scheduler's own log, not
   the toast). **Clicking to focus the window is not implemented**: `tauri-plugin-notification`'s
   desktop backend has no click/activation callback in its public API; real click-through needs
   the Windows toast API directly. See `docs/DECISIONS.md`'s step-4 entry.
5. **i18n growth**: `zh-CN.ts` fills in incrementally alongside each page (`onboarding`,
   `today`, `chat`, `settings`, `errors`) rather than all at once.

## M7.1 — Commands + events plumbing (no UI yet) — Done

All 25 commands in §10.2 are implemented in `commands.rs`, registered in `invoke_handler`, and
verified via real Tauri IPC (not just "compiles") in `apps/desktop/src-tauri/tests/commands.rs`.
Full reasoning in `docs/DECISIONS.md`'s M7.1 entry; summary:

- **Router reload closed** (M4's own "Open for M7" note): `CoreApp::reload_models()` rebuilds
  the LLM router from whatever `settings.models()` holds and swaps it in, so `save_models`/
  `set_api_key` take effect on the already-running app immediately — needed for onboarding's
  save-then-test-connection flow to work at all.
- **A real production bug found and fixed**: Tauri 2 denies every app command by default, and
  `capabilities/default.json` (from M0) never granted any. Every command would have been
  unreachable from the real webview, not just untested. Fixed via `build.rs`'s
  `AppManifest::commands(...)` (autogenerates `allow-<command>` permissions) plus granting each
  one in `capabilities/default.json`.
- **New core-side pieces**: `ConversationStore::recent_visible` (so `get_history`'s `limit`
  means "N visible messages," not "N raw rows before filtering"), `Settings::
  set_plan_time_and_day_boundary` (one atomic setter, avoiding an order-dependency hazard the
  two single-field setters have when both change together), `SecretStore::has` (default method,
  presence without exposing the key), `usage::summary` + `RoleUsage`, `CoreApp::get_history`/
  `delete_entry`/`log_weight`/`usage_summary`. `ChatTurn` and `MemoryNote` gained `Serialize`.
- **`get_history`'s activity-chip question resolved**: only a *live* `send_message` response
  carries `activities`; history replay doesn't reconstruct them (they aren't persisted).
- **`autostart`'s OS-level registration stays out of scope here** — `save_settings` only
  persists the flag; wiring `tauri_plugin_autostart` itself is M7.5's job.

**Done when** (met): every command in the §10.2 table compiles, is registered, and round-trips —
verified with 5 real IPC tests (`tauri::test`'s mock runtime, the project's actual `tauri.conf.json`
and `capabilities/`) plus 10 new `crates/core` unit tests. fmt/clippy/tests all clean, 312 tests
total. Manually ran the real binary against this machine's actual data dir: no panic.

## M7.2 — Onboarding UI — Done

Full three-step wizard (`ui/src/onboarding/`), gated in `App.tsx` on
`get_app_state().onboarding_complete`. Full reasoning in `docs/DECISIONS.md`'s M7.2 entry;
summary:

- **Step 1 (基本信息) / Step 2 (目标)**: all fields from §10.3, live `preview_targets` call in
  step 2 recomputed via a `useEffect` on every relevant draft field change.
- **Step 3 (模型)**: provider kind select with Anthropic prefill (base URL + suggested per-role
  models, exactly as §10.3 specifies), a "same model for all roles" toggle, 测试连接 testing all
  three roles and gating 完成 until a test against the *current* field values has succeeded (a
  `modelSignature` snapshot invalidates a stale pass if the user edits anything afterward).
- Data persists only at the end (完成: `save_profile`/`log_weight`/`save_weight_goal`/
  `complete_onboarding`) — 测试连接 itself already saves the candidate `models`/API key via
  `save_models`/`set_api_key` so `test_model` has something real to test against.
- **Two real bugs found and fixed** while actually building/running this (not just writing it):
  a `vite build` minifier crash on ordinary destructuring (fixed by dropping the unneeded
  `safari13` build target — this project is Windows-only), and a layout bug that pushed the
  navigation buttons off-screen for any step taller than the window (fixed with a scrollable
  content area + fixed footer).
- **`test_model` is never exercised against a real provider in automated tests** (CLAUDE.md: no
  real LLM calls in tests) — covered instead for the "nothing configured yet" case IPC-tests can
  safely exercise; real-provider testing is a manual check, same as M6's.

**Done when** (met): fresh data dir → onboarding runs end-to-end → `get_app_state` reports
`onboarding_complete: true, models_configured: true` and the main shell renders. Verified via
screenshots of the running app (all three steps, both model-step layouts) plus 4 new real-IPC
tests covering every onboarding command that didn't already have one; full click-through with
real mouse/keyboard input wasn't possible in this sandbox (no `xdotool` and no way to install it
without a password) — worth a real run-through on Windows before considering this fully closed.

## M7.3 — Today + Chat pages

- **Today**: kcal ring / macro bars / meal list off `TodayView.cards["diet"]`, plan cards with
  the four action buttons (接受计划 / 重新生成 / 调整), weight-log affordance. Worth a quick
  diff pass to confirm the diet plugin's `today_card` JSON shape already matches what the UI
  needs.
- **Chat**: message list from `get_history`; `send_message` on submit; activity chips come from
  the *current* turn's `ChatTurn.activities` — this resolves the M7.1 open question for the
  live case; only history replay of past turns stays ambiguous.
- Both pages refetch on `data-changed` / `plan-ready` / `plan-failed` events from the
  cross-cutting forwarding task.

**Done when**: manual run of acceptance-scenario steps 2–4 and 6–7 (§15) passes against a real
provider.

## M7.4 — Settings page

- Profile/goal forms reused from onboarding (same components, different entry point).
- Model settings: provider list, masked key display (`has_api_key` per provider, never `get`),
  role bindings, `test_model` buttons, `get_usage_summary` table.
- Plan timing, notifications/autostart toggles (`save_settings`), memory list with delete.

**Done when**: every field in §10.3's Settings section is editable and persists across an app
restart.

## M7.5 — Shell chrome: tray, single instance, autostart, quick-entry

- `tray.rs`: menu with the four items (打开 / 快速记录 / 生成明日计划 / 退出); close-to-tray via
  intercepting the main window's close event (`prevent_close` + `hide()`); 退出 calls
  `app_handle.exit(0)`.
- Single-instance callback focuses the existing window (already stubbed in `lib.rs`; just needs
  the real body).
- Autostart: Settings toggle drives `tauri_plugin_autostart`'s enable/disable, backed by the
  `autostart` setting.
- Quick-entry: a second `WebviewWindow` (small, always-on-top, no decorations), its own tiny
  route, submitting through `send_message` and closing/showing the reply after a short delay.

**Done when**: closing the main window leaves the process running in the tray; a second launch
focuses the existing window instead of opening a new one; quick-entry logs a message
end-to-end.

## M7.6 — Full acceptance pass

Run all seven steps of §15 manually against a real provider, per CLAUDE.md's "no real LLM in
automated tests" rule — this is the one sub-step that's manually verified rather than
test-suite verified, same as M6 was.

**Done when**: all seven §15 steps pass in one sitting against a real provider, with the result
noted in a commit message the way M6's manual verification was.

## Environment caveat and open decisions

The dev sandbox for this project is Linux (WSL2). The tray icon, Windows toast notifications,
autostart, and the keyring backend are all platform-specific. The Rust and React layers can be
built and unit-tested here, but M7.6's manual acceptance pass and the tray/notification/autostart
pieces of M7.5 need real verification on Windows — record this the same way M6's manual
verification note called out testing with a real API key.

Open decisions still to record in `docs/DECISIONS.md`:

- Whether `get_history` reconstructs activity chips for past turns (not persisted today) or
  omits them for history replay, keeping them only on the live `send_message` response.

Resolved: the `CommandError`/`auth` vs `llm` question — see the 2026-09-25 M7.1-step-1 entry.
