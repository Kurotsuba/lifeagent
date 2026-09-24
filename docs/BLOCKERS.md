# Phase 1 blockers — open decisions

One defect in `PHASE1_SPEC.md` that must be resolved before the milestone that depends on it can
close. It is **expression-level**: the surrounding module boundaries, function signatures and
data shapes in the spec are correct, so nothing needs redesigning — it needs no judgment call
either, just the fix below and the tests that prove it.

Record its resolution in `DECISIONS.md`, then amend the cited spec section so the spec stays the
source of truth (`CLAUDE.md`: "the spec wins for Phase 1 behavior").

Nothing here blocks M0–M4. The whole `llm` crate and all of `core` except `scheduler.rs` (and
`plan.rs`'s backup step) can be built and closed before this is settled.

**B1 (macro split could produce zero/negative carbs), B2 (fiber double-counted against carbs)
and B3 (scheduler catch-up misfired in the pre-boundary window) are resolved** — see
`DECISIONS.md`'s 2026-09-24 entries and `PHASE1_SPEC.md` §8.6/§8.3/§9.3/§7.13/§7.2 for the
corrected formulas and rules. Removed from this file; kept only in `DECISIONS.md` as the
historical record.

---

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

B4 gates M5's "done when: planning and scheduler integration tests pass" (B1, B2 and B3 also
did, until resolved — see above).
