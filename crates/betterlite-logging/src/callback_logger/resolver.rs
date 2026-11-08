use std::collections::BTreeMap;
use std::sync::Arc;

use super::CallbackLogger;
use crate::registry::error::RegistryError;

pub trait LoggerResolver: Send + Sync {
    fn resolve(
        &self,
        name: &str,
        params: &BTreeMap<String, String>,
    ) -> Result<Option<Arc<dyn CallbackLogger>>, RegistryError>;
}
