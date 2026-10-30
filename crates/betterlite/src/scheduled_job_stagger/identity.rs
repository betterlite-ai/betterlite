use std::collections::BTreeMap;
use std::process;

use uuid::Uuid;

// The PID suffix keeps two replicas sharing a hostname from landing on the same offset.
#[must_use]
pub fn resolve_stagger_identity(
    configured: Option<&str>,
    environment: &BTreeMap<String, String>,
) -> String {
    let host = configured
        .map(ToOwned::to_owned)
        .or_else(|| environment.get("POD_NAME").cloned())
        .or_else(|| environment.get("HOSTNAME").cloned())
        .unwrap_or_else(|| whoami::hostname().unwrap_or_else(|_err| Uuid::new_v4().to_string()));
    format!("{host}:{}", process::id())
}

mod tests;
