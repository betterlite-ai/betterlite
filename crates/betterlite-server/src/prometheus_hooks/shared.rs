use std::sync::{Arc, Mutex, PoisonError};

use super::hooks::PrometheusHooks;

// litellm_logging reuses the PrometheusLogger already in the in-memory logger list, so the process keeps one
static SHARED: Mutex<Option<Arc<PrometheusHooks>>> = Mutex::new(None);

impl PrometheusHooks {
    // the first call runs the build, and every later call returns the instance it made
    pub fn shared<Build, BuildError>(build: Build) -> Result<Arc<Self>, BuildError>
    where
        Build: FnOnce() -> Result<Self, BuildError>,
    {
        let mut installed = SHARED.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(existing) = installed.as_ref() {
            return Ok(Arc::clone(existing));
        }
        let hooks = Arc::new(build()?);
        *installed = Some(Arc::clone(&hooks));
        Ok(hooks)
    }
}

mod tests;
