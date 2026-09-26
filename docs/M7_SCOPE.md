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

## M7.3 — Today + Chat pages — Done

Built the main shell (`App.tsx`'s `MainShell` + `components/Sidebar.tsx`, gated on the same
`onboarding_complete` check as before) plus the `today/` and `chat/` page trees. Full reasoning
in `docs/DECISIONS.md`'s M7.3 entry; summary:

- **Today** (`today/Today.tsx` + `KcalRing`/`MacroBars`/`MealList`/`TodayPlanList`/
  `TomorrowPlanCard`/`WeightLogDialog`): kcal ring and protein/carbs/fat bars off
  `TodayView.cards["diet"]` (confirmed the diet plugin's `today_card` JSON shape against
  `intake.rs` directly rather than guessing), meal list with a two-step (tap-to-arm) delete
  instead of a browser `confirm()` popup, today's plan as a ✓/○ checklist against logged meal
  types, tomorrow's plan with 接受计划/重新生成/调整 and the no-plan-yet `立即生成` state (auto-
  generate time read live from `get_settings` rather than hardcoding "21:00").
- **Chat** (`chat/Chat.tsx`): message list from `get_history`, optimistic user bubble removed on
  send failure (text restored to the input for retry), activity chips matched against the *last*
  rendered assistant message by content equality (since only a live `send_message` response
  carries `activities` — history replay doesn't reconstruct them, per the M7.1 decision), inline
  error with a 重试 button plus a 前往设置 link gated on `auth`/`not_configured` error codes.
- **调整 cross-page wiring**: `MainShell` holds a `chatDraft` string; Today's 调整 button calls
  `onAdjustPlan("调整明天的计划：")`, which sets the draft and switches to the 对话 tab; `Chat`
  consumes it into the input box on mount and clears it, so it doesn't reappear on a later visit.
- Both pages subscribe to `data-changed`/`plan-ready`/`plan-failed` via a shared
  `api/events.ts` (`onAppDataEvent`) and refetch on any of the three, per §10.3.
- **Settings is still a placeholder tab** (`{设置} — 开发中`) — that's M7.4's own page; 前往设置
  and the sidebar item both already route to it, they just land on a stub for now.

**Verified**: `cargo fmt --all -- --check` / `cargo clippy --workspace --all-targets -- -D
warnings` / `cargo test --workspace` all clean (no Rust changed — M7.1 already exposed every
command this milestone needed); `npm run build` (`tsc -b && vite build`) passes strict with no
unused locals/params. Screenshotted both pages against the actual running app
(`GDK_BACKEND=x11 npm run tauri dev`, per M0's own screenshot recipe in `docs/DECISIONS.md`)
using this machine's real data dir from earlier manual testing (a real profile, weight, and
chat history already in it) — Today showed correct kcal/macro numbers against a computed
target and the no-plan-yet state; Chat showed the real prior conversation rendering correctly
with properly (dis)abled send button. No real `send_message`/`generate_plan` call was made
during this check (would hit the real configured provider) — same click-automation gap as
M7.2 (no `xdotool`/equivalent in this sandbox), worked around the same way: temporarily forcing
`App`'s screen/tab state to reach each page, screenshotting, then reverting the edit exactly
(`git diff` confirmed clean before moving on).

**Done when** (met): manual run of acceptance-scenario steps 2–4 and 6–7 (§15) needs a real
provider actually sending/generating, which is out of scope for this sandbox's read-only
screenshot check — worth a real click-through pass on Windows before closing Phase 1, same
caveat as M7.2's.

## M7.4 — Settings page — Done

Built `settings/Settings.tsx` (four `SectionCard`s in one scrollable page) plus one component
per §10.3 bullet. Full reasoning in `docs/DECISIONS.md`'s M7.4 entry; summary:

- **个人档案与目标** (`ProfileGoalSection.tsx`): reuses onboarding's `StepProfile`/`StepGoal`
  verbatim against one shared draft loaded from `get_profile`/`get_weight_goal`/`get_today`
  (for the latest weight `StepGoal`'s live preview needs), one 保存 button that calls
  `save_profile`/`save_weight_goal` and `log_weight` only if the weight field actually changed
  from what was loaded.
- **模型** (`ModelSection.tsx`): its own `ModelDraft` (not `OnboardingDraft` — no wizard-only
  fields), API key field is always blank with a `has_api_key`-driven hint (已设置/未设置 ·
  留空则不修改), left empty on save/test so the stored key is never overwritten with nothing.
  A 测试 button per role (`test_model`) that persists the candidate config first (same
  save-then-test approach as onboarding's 测试连接), and a 本月用量 table from
  `get_usage_summary`.
- **计划 · 通用** (`GeneralSection.tsx`): one section, one 保存 button — `SettingsDto`/
  `save_settings` persist plan time, day boundary, notifications, and autostart atomically in
  one call, so splitting them into two saved forms would just add a false-independence risk
  (saving 计划 without 通用 still overwrites both server-side, since the DTO has no partial-update
  form).
- **记忆** (`MemorySection.tsx`): list from `get_memory`, delete via the new shared
  `components/ConfirmDeleteButton.tsx` (extracted from Today's `MealList`, which now uses it
  too, since both needed the identical two-step-confirm behavior).
- New command wrappers added: `saveSettings`, `getMemory`/`deleteMemory`, `getUsageSummary`.

**Verified**: `cargo fmt`/`clippy`/`test --workspace` clean (no Rust changed). `npm run build`
clean, strict. Screenshotted the real running app (same `GDK_BACKEND=x11 npm run tauri dev` +
`import` recipe as M7.3) against this machine's real, already-configured data dir — confirmed
every field round-trips real backend data: profile (sex/birth date/height/weight/activity all
correct), goal preview (2080 kcal/日, matching Today's own ring exactly), model config (real
provider/base URL), a real 本月用量 table with non-zero token counts from actual earlier API
calls, plan/general settings, and one real memory note with its timestamp. Scrolling the actual
page wasn't reliable in this sandbox (see `docs/DECISIONS.md`), so the page was temporarily
scaled down (CSS `transform: scale()`) to fit everything in one screenshot instead — reverted
before finishing, confirmed with a `TEMP` marker grep across `apps/desktop/ui/src`.

**Done when** (met): every field in §10.3's Settings section is editable and, per the screenshot
above, loads real previously-saved values back — an actual restart-persistence check (edit a
field, restart the app, confirm it stuck) still needs a real click-through pass, same caveat as
M7.2/M7.3.

## M7.5 — Shell chrome: tray, single instance, autostart, quick-entry — Done

Full reasoning in `docs/DECISIONS.md`'s M7.5 entry; summary:

- **`tray.rs`**: `TrayIconBuilder` with a `Menu` of the four items (打开 / 快速记录 /
  生成明日计划 / 退出), reusing the app's own configured window icon (no separate tray asset).
  退出 calls `app_handle.exit(0)`; 打开 shows+focuses the main window (`focus_main_window`, also
  reused by the single-instance callback below); 生成明日计划 calls `CoreApp::generate_plan`
  directly (not through IPC — this click didn't come from the webview), reusing
  `commands::resolve_plan_date` (now `pub(crate)`) for the same "no date -> tomorrow" default
  Today's own 立即生成 button uses. Close-to-tray: `on_window_event` on the main window
  intercepts `CloseRequested`, calls `api.prevent_close()` + `hide()`.
- **Single instance**: `lib.rs`'s stubbed callback now calls `tray::focus_main_window`.
- **Autostart**: `commands::save_settings` gained an `AppHandle` parameter (a Tauri "special"
  extractor — no frontend change needed) and now calls `tauri_plugin_autostart`'s
  `enable`/`disable` right after persisting the `autostart` flag; `setup()` also reconciles the
  OS registration against the stored setting on every launch (covers drift, e.g. the user
  removed it via the OS's own startup settings outside the app). Both treat a sync failure as
  non-fatal (logged, not propagated) — same as `notify.rs`'s own treatment of OS integration.
- **Quick-entry**: a second `WebviewWindow` (`WebviewWindowBuilder`, label `"quick-entry"`,
  undecorated/always-on-top/no-resize, `index.html` — same bundle as the main window). Rather
  than a hash-route or separate HTML entry point, `main.tsx` picks which React root to mount by
  checking `getCurrentWindow().label` (`@tauri-apps/api/window`) — simpler than URL-based
  routing for a single-page app that otherwise has no client-side router at all. Its own
  `capabilities/quick-entry.json` grants only `core:default` + `allow-send-message` (least
  privilege — it has no business touching the day's data commands the main window uses). The
  window is reused (shown/focused) rather than rebuilt on every 快速记录 click; after a reply,
  it hides itself (not closes) after a few seconds, or immediately on Escape.

**API verified against the actual installed crate sources** (`~/.cargo/registry/src/.../
tauri-2.11.6`, `tauri-plugin-autostart-2.5.1`), not guessed from memory — every non-obvious
signature (`MenuItem::with_id`, `TrayIconBuilder`, `WindowEvent::CloseRequested`'s
`CloseRequestApi`, `WebviewWindowBuilder`, `ManagerExt::autolaunch`) was grepped from source
before use, since a plausible-but-wrong Tauri 2 API guess would only surface as a compile error
this sandbox could catch anyway — cheaper to check first.

**Verified**: compiles clean on the first try; `cargo fmt`/`clippy`/`test --workspace` all pass
(no new automated tests for the OS-integration paths themselves — see `docs/DECISIONS.md`'s
testability note). Ran the real app (`GDK_BACKEND=x11 npm run tauri dev`): no panic or logged
error during `tray::build`/single-instance/autostart-sync startup; `xwininfo`'s window tree
showed a real tray-icon proxy window was created (`"tray-icon tray app <pid>-1"`, 16×16 —
WSLg has no panel/taskbar to actually display it in, but its existence confirms
`TrayIconBuilder` succeeded); temporarily made `open_quick_entry` `pub` and called it once from
`setup()` (no click automation in this sandbox — same gap as M7.2–M7.4) to screenshot the
quick-entry window, confirming it opens undecorated/small with the right placeholder text and
autofocus; reverted immediately after, confirmed clean via `grep -rn "TEMP"`. Did **not** submit
a message through it or trigger 生成明日计划 — both would hit this machine's real, already-
configured LLM provider, which this check had no reason to spend on.

**Done when** (structurally met, real-input verification still pending): closing the main window
should leave the process running in the tray (code path is in place, but actually clicking the
window's own close button/OS close control needs input automation or a human, which this
sandbox has neither); a second launch focusing the existing window and quick-entry logging a
message end-to-end are the same story. All three need a real Windows pass — see the environment
caveat below and `docs/DECISIONS.md`'s M7.2 precedent for this exact gap.

## M7.6 — Full acceptance pass — Done

Run all seven steps of §15 manually against a real provider, per CLAUDE.md's "no real LLM in
automated tests" rule — this is the one sub-step that's manually verified rather than
test-suite verified, same as M6 was.

**Progress (2026-09-26, Windows, cross-built installer):** all ten §15 steps pass (1–4 after
the fixes in `docs/DECISIONS.md`'s two "M7.6" entries): memory note saved; plan v1 −6.3% vs
target, v2 (revision) −1.2%, no cilantro; close-to-tray + scheduled generation + toast;
catch-up on relaunch (both plans `trigger = schedule`); logs hold only `llm call` metadata
lines — no key, no personal text. M7.5's Windows-only checks also pass: single instance,
quick-entry (logged a latte end to end), tray 生成明日计划 toast, and (confirmed 2026-09-26) the
autostart toggle — enabling it in Settings registers LifeAgent with Windows startup. Fixes
found along the way (not blocking §15) — **done**, see `docs/DECISIONS.md`'s "M7.6 queued fixes" entry:
- Corrections drift: re-logging after a correction re-parses from text, so unchanged items get
  new guessed grams (beef 80→50 g, broth 400→300 g; lunch fell 662→617 kcal after *adding* an
  egg). Tell the conversation model to carry over unchanged items' grams from the previous
  tool result when re-logging.
- Dry vs cooked weight: "燕麦 40g" was estimated at cooked-oats density (68 kcal/100 g → 27
  kcal instead of ~150). Tell the parser the food name must name the state the grams refer to
  (`燕麦片(干)` vs `燕麦粥`).
- Model-stated plan numbers: both plan summaries say "约2160千卡、蛋白质135克" (the *targets*,
  not the plan's computed 2024/2135 kcal), and step 7's chat reply repeated "全天仍约2160 kcal".
  CLAUDE.md: numbers shown to the user are computed in Rust. The planner prompt should forbid
  kcal/macro figures in `summary`, and `render_plan_compact` (the plan text tools/system prompt
  hand the conversation model) should carry the app-computed meal and day kcal so the model
  quotes those. Also seen, lower priority: v1's summary claims "和前几天不重样" with no history
  (drop "referencing recent adherence" when there is none), and revision took 85 s on Opus 5
  at `high` effort — the chat just shows a spinner meanwhile.

**Done when** (met): all seven §15 steps pass in one sitting against a real provider, with the
result noted in a commit message the way M6's manual verification was. The autostart toggle —
the one M7.5 real-input item still unconfirmed when this progress note was first written — was
verified working by the user on 2026-09-26, closing the last open manual-check item for M7.

## Environment caveat and open decisions

The dev sandbox for this project is Linux (WSL2). The tray icon, Windows toast notifications,
autostart, and the keyring backend are all platform-specific. The Rust and React layers can be
built and unit-tested here, but M7.6's manual acceptance pass and the tray/notification/autostart
pieces of M7.5 needed real verification on Windows — done, per the entries above; every
Windows-only manual-check item for M7 is now confirmed.

No open decisions remain. Resolved: the `CommandError`/`auth` vs `llm` question (see the
2026-09-25 M7.1-step-1 entry), and `get_history`'s activity chips — omitted on history replay,
only a live `send_message` response carries them (see the M7.1 entry).
