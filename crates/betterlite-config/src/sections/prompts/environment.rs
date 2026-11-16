use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::loader::{ConfigError, EnvironmentLookup};

// LiteLLM's DEFAULT_MAX_RECURSE_DEPTH
const MAX_DEPTH: usize = 100;
// the config root and the prompts list item each add one level
const ENTRY_DEPTH: usize = 1;
// the litellm_params mapping adds one more
const PARAMS_DEPTH: usize = ENTRY_DEPTH + 1;
const ENV_PREFIX: &str = "os.environ/";
const VERSION_KEY: &str = "version";
const PARAMS_KEY: &str = "litellm_params";
// the bool fields of litellm_params that get_secret types
const FLAG_KEYS: [&str; 2] = [
    "ignore_prompt_manager_model",
    "ignore_prompt_manager_optional_params",
];

// LiteLLM resolves the whole config before it reads prompts, so this section resolves each entry itself
pub(super) fn resolve_entry<L>(
    entry: IndexMap<String, Json>,
    lookup: &L,
) -> Result<IndexMap<String, Json>, ConfigError>
where
    L: EnvironmentLookup,
{
    let mut resolved = IndexMap::with_capacity(entry.len());
    for (key, value) in entry {
        let value = match key.as_str() {
            VERSION_KEY => resolve_typed(&key, value, ENTRY_DEPTH, lookup)?,
            PARAMS_KEY => match value {
                Json::Object(params) => Json::Object(resolve_params(params, lookup)?),
                other => resolve_value(other, ENTRY_DEPTH, lookup),
            },
            _ => resolve_value(value, ENTRY_DEPTH, lookup),
        };
        resolved.insert(key, value);
    }
    Ok(resolved)
}

fn resolve_params<L>(
    params: IndexMap<String, Json>,
    lookup: &L,
) -> Result<IndexMap<String, Json>, ConfigError>
where
    L: EnvironmentLookup,
{
    let mut resolved = IndexMap::with_capacity(params.len());
    for (key, value) in params {
        let value = if FLAG_KEYS.contains(&key.as_str()) {
            resolve_typed(&key, value, PARAMS_DEPTH, lookup)?
        } else {
            resolve_value(value, PARAMS_DEPTH, lookup)
        };
        resolved.insert(key, value);
    }
    Ok(resolved)
}

// get_secret turns true and false into a bool and pydantic reads a number string into the int field, so a
// reference on a typed key goes through the shared env parser, and a bad value names its variable
fn resolve_typed<L>(key: &str, value: Json, depth: usize, lookup: &L) -> Result<Json, ConfigError>
where
    L: EnvironmentLookup,
{
    let text = match value {
        Json::Text(text) => text,
        other => return Ok(resolve_value(other, depth, lookup)),
    };
    let Some(name) = text.strip_prefix(ENV_PREFIX) else {
        return Ok(Json::Text(text));
    };
    let variable = lookup.lookup(name).unwrap_or_default();
    if key == VERSION_KEY {
        let version = betterlite_env::number::<i64>(name, &variable)?;
        return Ok(version.map_or(Json::Null, |number| Json::Number(number.into())));
    }
    Ok(betterlite_env::flag(name, &variable)?.map_or(Json::Null, Json::Bool))
}

fn resolve_mapping<L>(
    mapping: IndexMap<String, Json>,
    depth: usize,
    lookup: &L,
) -> IndexMap<String, Json>
where
    L: EnvironmentLookup,
{
    if depth > MAX_DEPTH {
        return mapping;
    }
    let mut resolved = IndexMap::with_capacity(mapping.len());
    for (key, value) in mapping {
        resolved.insert(key, resolve_value(value, depth, lookup));
    }
    resolved
}

// LiteLLM resolves mappings and the dict items of a list, never a plain list item, up to the depth limit
fn resolve_value<L>(value: Json, depth: usize, lookup: &L) -> Json
where
    L: EnvironmentLookup,
{
    let nested = depth.saturating_add(1);
    match value {
        Json::Object(mapping) => Json::Object(resolve_mapping(mapping, nested, lookup)),
        Json::List(items) => Json::List(
            items
                .into_iter()
                .map(|item| match item {
                    Json::Object(mapping) => Json::Object(resolve_mapping(mapping, nested, lookup)),
                    other => other,
                })
                .collect(),
        ),
        Json::Text(text) => resolve_text(text, lookup),
        other @ (Json::Null | Json::Bool(_) | Json::Number(_)) => other,
    }
}

// a missing variable becomes null, and docs/compatibility.md counts an empty one as unset
fn resolve_text<L>(text: String, lookup: &L) -> Json
where
    L: EnvironmentLookup,
{
    let Some(name) = text.strip_prefix(ENV_PREFIX) else {
        return Json::Text(text);
    };
    lookup
        .lookup(name)
        .filter(|value| !value.is_empty())
        .map_or(Json::Null, Json::Text)
}
