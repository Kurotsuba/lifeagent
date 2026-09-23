# Phase 1 blockers — open decisions

Four defects in `PHASE1_SPEC.md` that must be resolved before the milestones that depend on
them can close. All four are **expression-level**: the surrounding module boundaries, function
signatures and data shapes in the spec are correct, so nothing needs redesigning. Each is a few
lines of formula plus the tests that prove it.

Two (B2, B4) have one obvious fix and need only a recorded decision. Two (B1, B3) contain a real
judgment call, marked **DECIDE**.

Record every resolution in `DECISIONS.md`, then amend the cited spec section so the spec stays
the source of truth (`CLAUDE.md`: "the spec wins for Phase 1 behavior").

Nothing here blocks M0–M3. The whole `llm` crate and all of `core` except `plan.rs` and
`scheduler.rs` can be built and closed before any of this is settled.

---

## B1 — Macro split can produce zero or negative carbs

**Gates:** M4 (close), M5 (all) · **Spec:** §8.6, `PHASE1_SPEC.md:789` (formula at :808-812)

`protein_g = w × 2.0` (Lose) together with the `fat_g ≥ 0.6·w` floor can consume the entire kcal
budget, leaving `carbs_g ≤ 0`. Worked case: female, 150 kg, 155 cm, 60 y → BMR 2008, kcal floored
to 2010; protein 300 g (1200 kcal) + fat 90 g (810 kcal) = 2010 → carbs 0. The `if carbs_g < 100`
repair takes `max(0.6·w, …)`, so it returns the same 90 g of fat and fixes nothing.

Root cause: protein is anchored to *current* body weight, which is the wrong anchor for a user
well above their goal weight — and no rule guarantees the budget can satisfy protein, the fat
floor and a carb minimum simultaneously.

- [ ] **DECIDE** the protein anchor. Options:
  - **(a) Feasibility cap (recommended).** Keep `w × factor`, but clamp it so a carb minimum
    always survives: `protein_cap = (kcal − 4·carb_min − 9·fat_floor) / 4`, then
    `protein_g = min(w × factor, protein_cap)`. Smallest change, fixes the arithmetic directly,
    no new profile inputs.
  - **(b) Goal-weight anchor.** Use `target_weight_kg` when set, else `w`. Nutritionally the
    more standard choice, but it does nothing when no target is set (`target_weight_kg` is
    `Option`), so it needs (a) as a backstop anyway.
  - **(c) Adjusted body weight.** `ideal + 0.4·(actual − ideal)`. Most defensible clinically,
    most code, and it needs an ideal-weight formula the spec doesn't currently have.
- [ ] **DECIDE** what happens when even the cap is infeasible (protein would fall below, say,
      1.2 g/kg). Raise `kcal` to the feasible minimum and set `floor_applied = true`, or accept
      the lower protein?
- [ ] Rewrite the `if carbs_g < 100` repair branch so it can actually lower fat, or delete it if
      option (a) makes it unreachable.
- [ ] Define `round_to_10` (round vs. floor — it matters exactly at the kcal floor).
- [ ] Amend §8.6; record in `DECISIONS.md`.
- [ ] Tests (§13 already requires most of these): male/female, each goal direction, rate cap,
      floor application, carbs-minimum adjustment, goal-reached → maintain, **plus at least one
      hand-computed example** and **the 150 kg case above, asserting `carbs_g > 0`**.

## B2 — Fiber double-counted against total carbohydrate

**Gates:** M4 (close) · **Spec:** §8.3, `PHASE1_SPEC.md:751`

The per-100 g check treats fiber as separate from carbs — `protein + carbs + fat + fiber ≤ 100`,
and `e = 4·protein + 4·carbs + 9·fat + 2·fiber`. Food composition tables report fiber *inside*
total carbohydrate. Wheat bran (p 15.5, c 64.5, f 4.3, fiber 42.8) sums to 127 → rejected →
re-asked → hard-fails with `无法估算该食物营养`. The energy formula overcounts the same grams twice.

The spec never states which convention `carbs_g` uses, and neither does the §9.3 estimator prompt
(`PHASE1_SPEC.md:973`) — so today the model's choice is a coin flip.

- [ ] Declare `carbs_g` = **total carbohydrate, fiber included** (recommended: it is what food
      tables report, so it is what the LLM will produce by default).
- [ ] Sum rule → `protein + carbs + fat ≤ 100` (fiber no longer a separate addend).
- [ ] Energy rule → `e = 4·protein + 4·(carbs − fiber) + 9·fat + 2·fiber`.
- [ ] Add validity rule `fiber_g ≤ carbs_g`.
- [ ] **State the convention in the §9.3 prompt explicitly** — this is the half that actually
      controls what the model returns; changing only the validator leaves the ambiguity intact.
- [ ] Check §8.6's `fiber_g = 14·kcal/1000` target and §8.10's "fiber < 70% of target" warning
      still read correctly under the chosen convention.
- [ ] Amend §8.3 + §9.3; record in `DECISIONS.md`.
- [ ] Tests: sanity check and kcal replacement (§13), **plus a high-fiber food (wheat bran) that
      passes**, and a `fiber > carbs` input that is rejected.

## B3 — Scheduler catch-up misfires in the pre-boundary window

**Gates:** M5 (close) · **Spec:** §7.13, `PHASE1_SPEC.md:684` (rule at :688-691)

The rule compares against "**today's calendar date** at plan_time", but between `day_boundary`
and midnight the logical day and the calendar day differ. Relaunch at 01:00 with plan_time 21:00:
logical today = the 23rd, tomorrow = the 24th, no plan exists — but the test compares against the
24th at 21:00, which hasn't passed, so no catch-up runs. This is acceptance scenario §15.9.

- [ ] Anchor the comparison to the **logical** day: `due_at` = local datetime of
      (`logical_today` at `plan_time`); generate when `local_now ≥ due_at` and no plan exists for
      `logical_today + 1`.
- [ ] **DECIDE** `plan_time < day_boundary`, which the spec explicitly leaves undefined.
      Recommended: forbid it in settings validation (require `plan_time ≥ day_boundary`) rather
      than defining a second anchoring rule — one less branch to test.
- [ ] **DECIDE** the gap case: after downtime, the *current* logical day never gets a plan at all
      (catch-up only ever targets tomorrow). Leave it to the 立即生成 button, or generate for
      today too? Recommended: leave it — but record the choice, because it is silent behavior.
- [ ] Amend §7.13; record in `DECISIONS.md`.
- [ ] Tests: "is it time" across several `plan_time` / `day_boundary` combinations (§13), **plus
      the 01:00 relaunch case above**, plus the catch-up integration test with `FixedClock`.

## B4 — Daily backup is unsafe under WAL

**Gates:** M5 (close) · **Spec:** §12, `PHASE1_SPEC.md:1145-1146`

The DB is opened in WAL mode, and the backup copies `lifeagent.db` alone. Recent commits live in
the `-wal` file, so the copy is stale or torn. The same sentence also contradicts itself: one
fixed filename `lifeagent.db.bak`, but "keep last 7 dated copies".

- [ ] Replace the file copy with `VACUUM INTO 'backups/lifeagent-YYYY-MM-DD.db'` — atomic,
      WAL-correct, and it compacts. (`sqlite3_backup` API is the alternative; `VACUUM INTO` is
      one statement through `sqlx`.)
- [ ] Fix the naming contradiction: dated filenames, retain the 7 newest, delete the rest.
- [ ] Amend §12; record in `DECISIONS.md`.
- [ ] Test: run a backup, open the result, assert the last-written row is present.

---

## Suggested order

1. **B2, B4** — no real judgment, resolve and record now; they stop being a decision and become
   two small spec edits.
2. **B3** — one mechanical fix plus two policy calls, both with defaults.
3. **B1** — the only genuine design decision. It gates M4's close, so it has all of M0–M3 as
   runway, but it also drives §8.9/§8.10 (plan input and validator) and two leaf features that
   call the engine directly: the CLI's `targets` subcommand (§11) and the desktop's
   `preview_targets` command (§10.2), which onboarding step 2 uses to show the kcal target live.

B1 and B2 both gate M4's "done when: diet unit tests pass". B1, B3 and B4 gate M5's "done when:
planning and scheduler integration tests pass".
