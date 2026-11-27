use std::sync::{Arc, Mutex};

use super::hooks::PrometheusHooks;

// litellm_logging reuses the PrometheusLogger already in the in-memory logger list, so the process keeps one
pub(super) static SHARED: Mutex<Option<Arc<PrometheusHooks>>> = Mutex::new(None);

mod tests;
