use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use crate::loader::{ConfigError, EnvironmentLookup};

// LiteLLM's DEFAULT_MAX_RECURSE_DEPTH
const MAX_DEPTH: usize = 100;
// the config root and the prompts list item each add one level
const ENTRY_DEPTH: usize = 1;
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
    mut entry: IndexMap<String, Json>,
    lookup: &L,
) -> Result<IndexMap<String, Json>, ConfigError>
where
    L: EnvironmentLookup,
{
    if let Some(version) = entry.get_mut(VERSION_KEY) {
        resolve_typed(VERSION_KEY, version, lookup)?;
    }
    if let Some(Json::Object(params)) = entry.get_mut(PARAMS_KEY) {
        for key in FLAG_KEYS {
            if let Some(flag) = params.get_mut(key) {
                resolve_typed(key, flag, lookup)?;
            }
        }
    }
    Ok(resolve_mapping(entry, ENTRY_DEPTH, lookup))
}

// get_secret and pydantic type these references, so a bad value is a load error that names its variable
fn resolve_typed<L>(key: &str, field: &mut Json, lookup: &L) -> Result<(), ConfigError>
where
    L: EnvironmentLookup,
{
    let Json::Text(text) = &*field else {
        return Ok(());
    };
    let Some(name) = text.strip_prefix(ENV_PREFIX) else {
        return Ok(());
    };
    let variable = lookup.lookup(name).unwrap_or_default();
    let typed = if key == VERSION_KEY {
        let version = betterlite_env::number::<i64>(name, &variable)?;
        version.map_or(Json::Null, |number| Json::Number(Number::from(number)))
    } else {
        // a flag field is `bool | None = False` and PromptLiteLLMParams refuses null, so unset reads as false
        match variable.trim().to_ascii_lowercase().as_str() {
            "t" | "y" => Json::Bool(true),
            "f" | "n" => Json::Bool(false),
            _ => Json::Bool(betterlite_env::flag(name, &variable)?.unwrap_or(false)),
        }
    };
    *field = typed;
    Ok(())
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
                    other @ (Json::Null
                    | Json::Bool(_)
                    | Json::Number(_)
                    | Json::Text(_)
                    | Json::List(_)) => other,
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
