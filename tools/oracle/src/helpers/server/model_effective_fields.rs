mod answers;
mod support;

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
    StrategyRouterWrite(ParamsPair),
    // LiteLLM also runs its tuning quota with no license limit here; betterlite has no quota to run
    TuningWrite(ParamsPair),
    EffectiveParams(ParamsPair),
    MemberMarker(MarkerCall),
    CredentialName(CredentialCall),
    RateLimits(RateLimitCall),
    Ptu(PtuCall),
}

#[derive(Debug, Deserialize)]
struct ParamsPair {
    incoming: Option<JsonObject>,
    existing: Option<JsonObject>,
}

#[derive(Debug, Deserialize)]
struct MarkerCall {
    incoming: Option<JsonObject>,
    existing_params: JsonObject,
    existing_model_info: JsonObject,
}

#[derive(Debug, Deserialize)]
struct CredentialCall {
    incoming: Option<JsonObject>,
    existing: Option<JsonObject>,
    loaded: Vec<String>,
    stored: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RateLimitCall {
    litellm_params: JsonObject,
    enforced: bool,
}

#[derive(Debug, Deserialize)]
struct PtuCall {
    stored: JsonObject,
    incoming: Option<JsonObject>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        answer(input)
    }
}
