use betterlite_prompts::arguments::json::{Json, Object};

use super::{VCAP_AICORE_SERVICE_NAME, VCAP_SERVICES_ENV_VAR};
use crate::sap_auth::environment::SapEnvironment;

pub(super) fn load_vcap(environment: &dyn SapEnvironment) -> Option<Object> {
    let raw = environment.var(VCAP_SERVICES_ENV_VAR)?;
    if raw.is_empty() {
        return None;
    }
    serde_json::from_str::<Object>(&raw).ok()
}

pub(super) fn find_vcap_service(services: &Object) -> Option<&Json> {
    for (_, list) in services {
        let Json::List(entries) = list else {
            tracing::warn!("a VCAP_SERVICES service list is not a JSON array; skipping it");
            continue;
        };
        for entry in entries {
            let Json::Object(fields) = entry else {
                continue;
            };
            if fields.get("label").and_then(Json::as_str) == Some(VCAP_AICORE_SERVICE_NAME) {
                return Some(entry);
            }
        }
    }
    None
}
