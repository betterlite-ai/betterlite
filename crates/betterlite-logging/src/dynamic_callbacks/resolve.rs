use std::collections::BTreeMap;
use std::sync::Arc;

use secrecy::ExposeSecret as _;

use crate::callback_logger::CallbackLogger;
use crate::callback_logger::resolver::LoggerResolver;
use crate::logging_privacy::callbacks::TrustedCallbacks;

use super::error::DynamicCallbackError;

const DATADOG_PREFIX: &str = "dd_";
const NEWRELIC_PREFIX: &str = "newrelic_";

pub fn resolve_callback_string(
    callback: &str,
    trusted: &TrustedCallbacks,
    resolver: &dyn LoggerResolver,
) -> Result<Vec<Arc<dyn CallbackLogger>>, DynamicCallbackError> {
    let prefix = match callback {
        "datadog" => Some(DATADOG_PREFIX),
        "newrelic" => Some(NEWRELIC_PREFIX),
        _ => None,
    };
    let mut params = BTreeMap::new();
    for (name, value) in &trusted.values {
        if prefix.is_some_and(|candidate| name.starts_with(candidate)) {
            params.insert(name.clone(), value.expose_secret().to_owned());
        }
    }
    let Some(first) = resolve(resolver, callback, &params)? else {
        return Ok(Vec::new());
    };
    // the per-team metrics logger comes from the trusted key; the global trace logger still gets this request
    let team_scoped = callback == "newrelic" && params.contains_key("newrelic_api_key");
    if team_scoped
        && let Some(second) = resolve(resolver, callback, &BTreeMap::new())?
        && !Arc::ptr_eq(&first, &second)
    {
        return Ok(vec![first, second]);
    }
    Ok(vec![first])
}

fn resolve(
    resolver: &dyn LoggerResolver,
    callback: &str,
    params: &BTreeMap<String, String>,
) -> Result<Option<Arc<dyn CallbackLogger>>, DynamicCallbackError> {
    let answer = resolver.resolve(callback, params);
    answer.map_err(|source| DynamicCallbackError::Unavailable {
        callback: callback.to_owned(),
        source,
    })
}
