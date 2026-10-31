use std::collections::BTreeMap;
use std::process;

use uuid::Uuid;

pub trait IdentityEnvironment: Send + Sync {
    fn hostname(&self) -> Option<String>;
    fn random_identity(&self) -> String;
    fn process_id(&self) -> u32;
}

#[derive(Debug, Default)]
pub struct SystemIdentityEnvironment;

impl IdentityEnvironment for SystemIdentityEnvironment {
    fn hostname(&self) -> Option<String> {
        whoami::hostname().ok()
    }

    fn random_identity(&self) -> String {
        Uuid::new_v4().to_string()
    }

    fn process_id(&self) -> u32 {
        process::id()
    }
}

// The PID suffix keeps two replicas sharing a hostname from landing on the same offset; an empty configured or environment value is skipped, matching Python's truthy `or` chain.
#[must_use]
pub fn resolve_stagger_identity(
    configured: Option<&str>,
    environment: &BTreeMap<String, String>,
    dependencies: &dyn IdentityEnvironment,
) -> String {
    let host = configured
        .map(ToOwned::to_owned)
        .filter(|text| !text.is_empty())
        .or_else(|| {
            environment
                .get("POD_NAME")
                .cloned()
                .filter(|text| !text.is_empty())
        })
        .or_else(|| {
            environment
                .get("HOSTNAME")
                .cloned()
                .filter(|text| !text.is_empty())
        })
        .unwrap_or_else(|| {
            dependencies
                .hostname()
                .unwrap_or_else(|| dependencies.random_identity())
        });
    format!("{host}:{}", dependencies.process_id())
}

mod tests;
