use betterlite_context::bedrock_model::{get_bedrock_base_model, strip_bedrock_routing_prefix};
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::names::{
    NameNormalizers, PotentialNamesInput, potential_model_names,
};
use betterlite_models::resolution::ResolutionContext;
use indexmap::IndexSet;

use crate::mantle_inputs::region::split_region_prefix;
use crate::model_routes::{ParamMap, ParamValue};

const NOVA_CANVAS_FLAG: &str = "supports_nova_canvas_image_edit";
const BEDROCK_PREFIX: &str = "bedrock/";

const NORMALIZERS: NameNormalizers = NameNormalizers {
    bedrock_base_model: get_bedrock_base_model,
    bedrock_routing_prefix: strip_bedrock_routing_prefix,
    mantle_region_split: split_region_prefix,
};

// The flag sits on a raw model_cost entry, which get_model_info drops, so this reads the map itself.
#[must_use]
pub fn supports_nova_canvas_edit(
    model: &str,
    model_cost: &ParamMap,
    resolution: &ResolutionContext<'_>,
    keys: &mut ModelCostKeys,
) -> bool {
    if model.is_empty() {
        return false;
    }
    for name in candidate_names(model, resolution) {
        let Some(key) = keys.get_model_cost_key(model_cost, &name) else {
            continue;
        };
        if entry_flag(model_cost, &key) {
            return true;
        }
    }
    false
}

// LiteLLM tries the name, the slash suffix, the base model and the resolution names, in that order.
fn candidate_names(model: &str, resolution: &ResolutionContext<'_>) -> IndexSet<String> {
    let mut names = IndexSet::new();
    add_name(&mut names, model.to_owned());
    if let Some((_, suffix)) = model.rsplit_once('/') {
        add_name(&mut names, suffix.to_owned());
        add_name(&mut names, format!("{BEDROCK_PREFIX}{suffix}"));
    }
    let base = get_bedrock_base_model(model);
    if !base.is_empty() && base != model {
        add_name(&mut names, base.clone());
        add_name(&mut names, format!("{BEDROCK_PREFIX}{base}"));
    }
    let potential = potential_model_names(&PotentialNamesInput {
        model,
        custom_llm_provider: None,
        resolution,
        normalizers: NORMALIZERS,
    });
    for name in [
        potential.combined_model_name,
        potential.combined_stripped_model_name,
        potential.stripped_model_name,
        potential.split_model,
    ] {
        add_name(&mut names, name);
    }
    names
}

fn add_name(names: &mut IndexSet<String>, name: String) {
    if !name.is_empty() {
        names.insert(name);
    }
}

// Python's `entry.get(flag) is True`: a null entry, a missing entry and a "true" string all say no.
fn entry_flag(model_cost: &ParamMap, key: &str) -> bool {
    match model_cost.get(key) {
        Some(ParamValue::Mapping(fields)) => {
            matches!(fields.get(NOVA_CANVAS_FLAG), Some(ParamValue::Bool(true)))
        }
        None
        | Some(
            ParamValue::Null
            | ParamValue::Bool(_)
            | ParamValue::Number(_)
            | ParamValue::Text(_)
            | ParamValue::List(_),
        ) => false,
    }
}
