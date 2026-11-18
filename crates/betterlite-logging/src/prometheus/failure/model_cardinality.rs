use betterlite_models::decisions::Json;

use super::{RequestedModelCatalog, UNRECOGNIZED_MODEL_LABEL};

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
            Some(UNRECOGNIZED_MODEL_LABEL.to_owned())
        }
    }
}

fn vouched_or_other(
    model: &str,
    router_originated: bool,
    catalog: Option<&dyn RequestedModelCatalog>,
) -> String {
    let Some(vouched) = catalog else {
        return signed(model, router_originated);
    };
    let recognized = vouched.is_recognized_model(model)
        || vouched.is_team_public_model(model)
        || vouched.matches_pattern_route(model);
    if recognized {
        return model.to_owned();
    }
    UNRECOGNIZED_MODEL_LABEL.to_owned()
}

// With no proxy router, only an SDK Router's own event trusts its deployment name.
fn signed(model: &str, router_originated: bool) -> String {
    if router_originated {
        return model.to_owned();
    }
    UNRECOGNIZED_MODEL_LABEL.to_owned()
}
