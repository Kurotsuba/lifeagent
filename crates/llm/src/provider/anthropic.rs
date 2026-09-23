//! Anthropic Messages provider.
//!
//! Filled in during M2 per `docs/PHASE1_SPEC.md` §6.2.2. Not reachable by pointing the
//! OpenAI-compatible provider at a different `base_url` — the wire format differs in message
//! shape, tool schema nesting, tool-result placement and parameter surface.
