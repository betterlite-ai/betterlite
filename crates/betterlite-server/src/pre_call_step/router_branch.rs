use betterlite_config::sections::settings::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object as JsonMap;
use betterlite_router::model_catalog_lookup::alias::AliasTarget;
use indexmap::IndexMap;
use serde_json::Number;

use crate::pre_call::failure::PreCallFailure;
use crate::pre_call::key::PreCallKey;
use crate::pre_call::ports::RouterView;
use crate::router_weight_references::weights::{RouterWeights, decode_router_weights};

use super::json::to_decision;

pub(super) async fn apply_router_settings<K>(
    data: &mut JsonMap,
    settings: &JsonMap,
    router: &dyn RouterView<K>,
    key: &K,
) -> Result<(), PreCallFailure>
where
    K: PreCallKey,
{
    data.insert(
        String::from("router_settings_override"),
        JsonValue::Object(settings.clone()),
    );
    data.insert(
        String::from("_router_weights"),
        router_weights(settings.get("weights")),
    );
    let requested = data.get("model");
    let Some(target) = alias_target(settings.get("model_group_alias"), requested) else {
        return Ok(());
    };
    router.authorize_alias(&target, key).await?;
    data.insert(String::from("model"), JsonValue::String(target));
    Ok(())
}

fn router_weights(weights: Option<&JsonValue>) -> JsonValue {
    let Some(weights) = weights else {
        return JsonValue::Null;
    };
    let mut holder = IndexMap::new();
    holder.insert(String::from("weights"), to_decision(weights));
    match decode_router_weights(&Json::Object(holder)) {
        Ok(Some(decoded)) => weights_json(&decoded),
        Ok(None) => JsonValue::Null,
        Err(_) => {
            tracing::warn!(
                "Ignoring invalid saved router weights; update team/key router_settings"
            );
            JsonValue::Null
        }
    }
}

fn weights_json(weights: &RouterWeights) -> JsonValue {
    let mut groups = JsonMap::new();
    for (group, members) in weights {
        let mut entries = JsonMap::new();
        for (model, weight) in members {
            let stored = Number::from_f64(weight.0).map_or(JsonValue::Null, JsonValue::Number);
            entries.insert(model.0.clone(), stored);
        }
        groups.insert(group.0.clone(), JsonValue::Object(entries));
    }
    JsonValue::Object(groups)
}

fn alias_target(aliases: Option<&JsonValue>, model: Option<&JsonValue>) -> Option<String> {
    let JsonValue::Object(entries) = aliases? else {
        return None;
    };
    let requested = model?.as_str()?;
    let entry = entries.get(requested)?;
    let resolved = match entry {
        JsonValue::String(name) => AliasTarget::Name(name.clone()),
        JsonValue::Object(item) => AliasTarget::Item {
            model: item.get("model").map(to_decision),
            hidden: None,
        },
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) => {
            AliasTarget::Unsupported(Json::Null)
        }
    };
    let target = resolved.resolved()?;
    if target == requested {
        return None;
    }
    Some(target.to_owned())
}
