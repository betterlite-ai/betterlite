mod answers;
mod tests;

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;

use self::answers::answer;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    salt_key: String,
    ptu_enabled: bool,
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
enum Call {
    Prices(PricesCall),
    Priced(PricedCall),
    Zeroed(ZeroedCall),
    Delta(DeltaCall),
    PricedDeployment(DeploymentCall),
    Catalog(CatalogCall),
}

#[derive(Debug, Deserialize)]
struct PricesCall {
    values: Vec<Json>,
}

#[derive(Debug, Deserialize)]
struct PricedCall {
    model_info: JsonObject,
    supplied: JsonObject,
}

#[derive(Debug, Deserialize)]
struct ZeroedCall {
    model_info: JsonObject,
    litellm_params: JsonObject,
    supplied: JsonObject,
}

#[derive(Debug, Deserialize)]
struct DeltaCall {
    stored_model_info: JsonObject,
    model_info: JsonObject,
    litellm_params: JsonObject,
    patch: PatchBody,
}

// the updateDeployment body; an absent or null blob patches nothing
#[derive(Debug, Deserialize)]
struct PatchBody {
    litellm_params: Option<JsonObject>,
    model_info: Option<JsonObject>,
}

#[derive(Debug, Deserialize)]
struct DeploymentCall {
    litellm_params: JsonObject,
    model_info: JsonObject,
}

#[derive(Debug, Deserialize)]
struct CatalogCall {
    cost_map: BTreeMap<String, JsonObject>,
    catalog: BTreeMap<String, JsonObject>,
    model: String,
    incoming: JsonObject,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        answer(input)
    }
}
