//! `LiteLLM`'s `sentry_scrubbing.py`, ported.
//!
//! Builds deny-list patterns, a bounded repr/JSON scrubber, and the pure settings
//! `build_sentry_init_options` assembles. Wiring a real Sentry SDK call around these
//! settings is `litellm_logging.py`'s job, out of scope here: this module only ever
//! returns data and scrubbed values.

pub mod pattern;
pub mod scrub;
pub mod settings;

mod fixture;
