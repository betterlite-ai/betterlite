use crate::catalog::ModelCatalog;
use crate::fireworks_catalog_metadata::get_model_cost_key;
use crate::model_catalog_keys::strip_model_name;

use super::info::ModelQuery;
use super::resolve::{BedrockNames, split_model};

pub struct Names {
    pub split_model: String,
    pub combined: String,
    pub stripped: String,
    pub combined_stripped: String,
    pub provider_prefixed: String,
    pub provider: Option<String>,
}

fn unprovided_names(catalog: &ModelCatalog, bedrock: &BedrockNames, model: &str) -> Names {
    let split = split_model(catalog, model);
    let stripped = strip_model_name(model, split.provider.as_deref(), bedrock.base_model);
    Names {
        split_model: split.model,
        combined: model.to_owned(),
        combined_stripped: stripped.clone(),
        stripped,
        provider_prefixed: model.to_owned(),
        provider: split.provider,
    }
}

fn provided_names(bedrock: &BedrockNames, model: &str, provider: &str) -> Names {
    let remainder = model.strip_prefix(&format!("{provider}/"));
    let split = remainder.unwrap_or(model);
    let stripped = strip_model_name(split, Some(provider), bedrock.base_model);
    Names {
        split_model: split.to_owned(),
        combined: if remainder.is_some() {
            model.to_owned()
        } else {
            format!("{provider}/{model}")
        },
        combined_stripped: format!("{provider}/{stripped}"),
        stripped,
        provider_prefixed: format!("{provider}/{model}"),
        provider: Some(provider.to_owned()),
    }
}

#[must_use]
pub fn potential_names(
    catalog: &ModelCatalog,
    bedrock: &BedrockNames,
    query: &ModelQuery<'_>,
) -> Names {
    let mut names = query.provider.map_or_else(
        || unprovided_names(catalog, bedrock, query.model),
        |provider| provided_names(bedrock, query.model, provider),
    );
    match names.provider.as_deref() {
        Some("bedrock" | "bedrock_converse") => {
            names.split_model = (bedrock.strip_routing_prefix)(&names.split_model);
        }
        Some("bedrock_mantle") => {
            names.split_model = (bedrock.strip_mantle_region)(&names.split_model);
            let stripped = strip_model_name(
                &names.split_model,
                Some("bedrock_mantle"),
                bedrock.base_model,
            );
            names.combined_stripped = format!("bedrock_mantle/{stripped}");
        }
        Some("fireworks_ai") => {
            names.provider_prefixed = get_model_cost_key(&names.split_model);
        }
        _ => {}
    }
    names
}
