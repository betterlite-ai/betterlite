use std::collections::BTreeMap;

use serde::de::{DeserializeOwned, Error as _};

use super::super::allowlists::{DIRECT_COMMON, DIRECT_SYNC_ONLY};
use super::super::coercion::{coerce_bool, coerce_float, coerce_int};
use super::super::environment::EnvironmentLookup;
use super::super::error::RedisSettingsError;
use super::super::model::nodes::{NodeInput, NodesInput};
use super::super::model::scalars::{PassthroughValue, Scalar};

pub(super) fn resolve_passthrough<E>(
    mut passthrough: BTreeMap<String, PassthroughValue>,
    env: &E,
) -> BTreeMap<String, PassthroughValue>
where
    E: EnvironmentLookup,
{
    for option in DIRECT_COMMON.iter().chain(DIRECT_SYNC_ONLY) {
        let name = format!("REDIS_{}", option.to_uppercase());
        if let Some(value) = env.lookup(&name) {
            passthrough
                .entry((*option).to_owned())
                .or_insert(PassthroughValue::Text(value));
        }
    }
    passthrough
}

pub(super) fn resolve_nodes<T, F>(
    overrides: Option<NodesInput<T>>,
    env_text: Option<String>,
    on_error: F,
) -> Result<Option<Vec<T::Resolved>>, RedisSettingsError>
where
    T: DeserializeOwned + NodeInput,
    F: Fn(serde_json::Error) -> RedisSettingsError,
{
    let input = overrides
        .filter(NodesInput::is_truthy)
        .or_else(|| env_text.map(NodesInput::Text));
    let parsed = match input {
        None => return Ok(None),
        Some(NodesInput::Value(nodes)) => nodes,
        Some(NodesInput::Text(text)) => serde_json::from_str(&text).map_err(&on_error)?,
    };
    parsed
        .into_iter()
        .map(|node| {
            let (host, raw_port) = node.into_parts();
            let port = coerce_int(Some(raw_port))
                .ok_or_else(|| on_error(serde_json::Error::custom("invalid node port")))?;
            Ok(T::from_parts(host, port))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

pub(super) trait Coerce: Sized {
    fn coerce(value: Option<Scalar<Self>>) -> Option<Self>;
}

impl Coerce for i64 {
    fn coerce(value: Option<Scalar<Self>>) -> Option<Self> {
        coerce_int(value)
    }
}

impl Coerce for f64 {
    fn coerce(value: Option<Scalar<Self>>) -> Option<Self> {
        coerce_float(value)
    }
}

impl Coerce for bool {
    fn coerce(value: Option<Scalar<Self>>) -> Option<Self> {
        coerce_bool(value)
    }
}

pub(super) fn resolve_scalar<T, E>(value: Option<Scalar<T>>, env: &E, var: &str) -> Option<T>
where
    T: Coerce,
    E: EnvironmentLookup,
{
    let raw = value.or_else(|| env.lookup(var).map(Scalar::Text));
    T::coerce(raw)
}
