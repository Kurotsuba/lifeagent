//! `complete_json`: JSON-mode helper with fence-stripping and a one-shot repair retry.
//!
//! Filled in during M2 per `docs/PHASE1_SPEC.md` §6.4. Load-bearing for the Anthropic
//! provider, which has no native JSON-mode equivalent to fall back on.
