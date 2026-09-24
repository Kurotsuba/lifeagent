# Phase 1 blockers — open decisions

**No open blockers.**

B1 (macro split could produce zero/negative carbs), B2 (fiber double-counted against carbs),
B3 (scheduler catch-up misfired in the pre-boundary window) and B4 (daily backup unsafe under
WAL) are all resolved. See the 2026-09-24 entries in `DECISIONS.md` and the amended sections
of `PHASE1_SPEC.md` (§7.2, §7.13, §8.3, §8.6, §9.3, §12).

If a new defect in the spec is found that must be settled before a milestone can close, add
it here with the milestone it gates, the spec section, and any decision it needs. Record the
resolution in `DECISIONS.md` and amend the spec, so the spec stays the source of truth
(`CLAUDE.md`: "the spec wins for Phase 1 behavior").
