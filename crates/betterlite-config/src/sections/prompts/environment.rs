use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::loader::EnvironmentLookup;

// LiteLLM's DEFAULT_MAX_RECURSE_DEPTH
const MAX_DEPTH: usize = 100;
// the config root and the prompts list item each add one level
pub(super) const ENTRY_DEPTH: usize = 1;
const ENV_PREFIX: &str = "os.environ/";

// LiteLLM resolves the whole config before it reads prompts, so this section resolves each entry itself
pub(super) fn resolve_mapping<L>(
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
    mapping
        .into_iter()
        .map(|(key, value)| (key, resolve_value(value, depth, lookup)))
        .collect()
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
