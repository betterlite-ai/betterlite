use betterlite_models::decisions::Json;
use betterlite_router::model_catalog_lookup::alias::AliasTarget;

// Adapt saved settings to B302; both auth normalization and pre-call use the same resolver.
#[must_use]
pub fn resolve_model_group_alias(aliases: Option<&Json>, requested: &str) -> Option<String> {
    let Json::Object(entries) = aliases? else {
        return None;
    };
    let entry = entries.get(requested)?;
    let resolved = match entry {
        Json::Text(name) => AliasTarget::Name(name.clone()),
        Json::Object(item) => AliasTarget::Item {
            model: item.get("model").cloned(),
            hidden: None,
        },
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {
            AliasTarget::Unsupported(Json::Null)
        }
    };
    resolved.resolved().map(str::to_owned)
}
