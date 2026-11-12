use std::collections::BTreeMap;

use betterlite_env::EnvError;

use super::error::GuardrailParamsError;
use super::keys::{BOOL_KEYS, NUMBER_KEYS};
use crate::loader::EnvironmentLookup;
use crate::sections::guardrail_identity::item::GuardrailValue;

const ENV_PREFIX: &str = "os.environ/";
// LiteLLM's DEFAULT_MAX_RECURSE_DEPTH
const MAX_DEPTH: usize = 100;
// the config root, the guardrails list item and the litellm_params mapping each add one level
pub(super) const PARAMS_DEPTH: usize = 2;

pub fn resolve_params<L>(
    params: BTreeMap<String, GuardrailValue>,
    lookup: &L,
) -> Result<BTreeMap<String, GuardrailValue>, GuardrailParamsError>
where
    L: EnvironmentLookup,
{
    let mut resolved = BTreeMap::new();
    for (key, value) in params {
        let reference = if let GuardrailValue::Text(text) = &value {
            text.strip_prefix(ENV_PREFIX).map(str::to_owned)
        } else {
            None
        };
        let typed = match reference {
            Some(name)
                if BOOL_KEYS.contains(&key.as_str()) || NUMBER_KEYS.contains(&key.as_str()) =>
            {
                typed_variable(&key, &name, lookup)?
            }
            Some(_) | None => resolve_value(value, PARAMS_DEPTH, lookup),
        };
        resolved.insert(key, typed);
    }
    Ok(resolved)
}

fn typed_variable<L>(
    key: &str,
    name: &str,
    lookup: &L,
) -> Result<GuardrailValue, GuardrailParamsError>
where
    L: EnvironmentLookup,
{
    let text = lookup.lookup(name).unwrap_or_default();
    if BOOL_KEYS.contains(&key) {
        return Ok(betterlite_env::flag(name, &text)?
            .map_or(GuardrailValue::Null, GuardrailValue::Boolean));
    }
    if key == "timeout" {
        let timeout = betterlite_env::number::<f64>(name, &text)?;
        if timeout.is_some_and(|seconds| !seconds.is_finite()) {
            return Err(GuardrailParamsError::Environment(EnvError::Number {
                name: name.to_owned(),
            }));
        }
        return Ok(timeout.map_or(GuardrailValue::Null, GuardrailValue::Decimal));
    }
    Ok(betterlite_env::number::<i64>(name, &text)?
        .map_or(GuardrailValue::Null, GuardrailValue::SignedInteger))
}

pub(super) fn resolve_map<L>(
    map: BTreeMap<String, GuardrailValue>,
    depth: usize,
    lookup: &L,
) -> BTreeMap<String, GuardrailValue>
where
    L: EnvironmentLookup,
{
    if depth > MAX_DEPTH {
        return map;
    }
    map.into_iter()
        .map(|(key, value)| (key, resolve_value(value, depth, lookup)))
        .collect()
}

fn resolve_value<L>(value: GuardrailValue, depth: usize, lookup: &L) -> GuardrailValue
where
    L: EnvironmentLookup,
{
    let nested = depth.saturating_add(1);
    match value {
        GuardrailValue::Object(map) => GuardrailValue::Object(resolve_map(map, nested, lookup)),
        GuardrailValue::Array(items) => GuardrailValue::Array(
            items
                .into_iter()
                .map(|item| match item {
                    GuardrailValue::Object(map) => {
                        GuardrailValue::Object(resolve_map(map, nested, lookup))
                    }
                    GuardrailValue::Null
                    | GuardrailValue::Boolean(_)
                    | GuardrailValue::SignedInteger(_)
                    | GuardrailValue::UnsignedInteger(_)
                    | GuardrailValue::Decimal(_)
                    | GuardrailValue::Text(_)
                    | GuardrailValue::Array(_) => item,
                })
                .collect(),
        ),
        GuardrailValue::Text(text) => {
            if let Some(name) = text.strip_prefix(ENV_PREFIX) {
                return lookup
                    .lookup(name)
                    .filter(|variable| !variable.is_empty())
                    .map_or(GuardrailValue::Null, GuardrailValue::Text);
            }
            GuardrailValue::Text(text)
        }
        GuardrailValue::Null
        | GuardrailValue::Boolean(_)
        | GuardrailValue::SignedInteger(_)
        | GuardrailValue::UnsignedInteger(_)
        | GuardrailValue::Decimal(_) => value,
    }
}
