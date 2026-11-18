use betterlite_models::decisions::Json;

use super::{RequestedModelCatalog, UNRECOGNIZED_REQUESTED_MODEL_LABEL};

// LiteLLM's _bounded_requested_model_label: a name the router vouches for keeps its label value, anything else collapses.
#[must_use]
pub fn bounded_requested_model_label(
    requested_model: Option<&Json>,
    router_originated: bool,
    catalog: Option<&dyn RequestedModelCatalog>,
) -> Option<String> {
    match requested_model? {
        Json::Null => None,
        Json::Text(model) if model.is_empty() => Some(model.clone()),
        Json::Text(model) => Some(vouched_or_other(model, router_originated, catalog)),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            Some(UNRECOGNIZED_REQUESTED_MODEL_LABEL.to_owned())
        }
    }
}

fn vouched_or_other(
    model: &str,
    router_originated: bool,
    catalog: Option<&dyn RequestedModelCatalog>,
) -> String {
    let Some(catalog) = catalog else {
        return signed(model, router_originated);
    };
    let recognized = catalog.is_recognized_model(model)
        || catalog.is_team_public_model(model)
        || catalog.matches_pattern_route(model);
    if recognized {
        return model.to_owned();
    }
    UNRECOGNIZED_REQUESTED_MODEL_LABEL.to_owned()
}

// With no proxy router, only an SDK Router's own event trusts its deployment name.
fn signed(model: &str, router_originated: bool) -> String {
    if router_originated {
        return model.to_owned();
    }
    UNRECOGNIZED_REQUESTED_MODEL_LABEL.to_owned()
}
