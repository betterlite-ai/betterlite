use betterlite_config::spend_storage::SpendUpdates;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::spend::failure::fields::{
    TrackingIdentity, request_tags_for_cost_tracking, should_track_cost_callback,
    write_spend_metadata_to_kwargs,
};
use betterlite_server::spend::failure::metadata::used_client_oauth_token;
use serde::Deserialize;

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    Stamp(StampInput),
    Patch(PatchInput),
    Tags(TagsInput),
    Eligibility(EligibilityInput),
}

// _proxy_stamped_used_client_oauth_token: only the route-selected bucket and only a bool.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StampInput {
    request_data: JsonObject,
    request_route: Option<String>,
}

// _write_spend_metadata_to_kwargs: the non-null key fields and tags fill only null destinations.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PatchInput {
    request_data: JsonObject,
    metadata: JsonObject,
}

// _get_request_tags_for_cost_tracking: a list-valued standard payload wins over metadata tags.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TagsInput {
    standard_logging_object: Option<JsonObject>,
    metadata: JsonObject,
}

// _should_track_cost_callback: the shared exact-true spend switch, then any non-null identity.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EligibilityInput {
    disable_spend_updates: bool,
    user_api_key: Option<String>,
    user_id: Option<String>,
    team_id: Option<String>,
    end_user_id: Option<String>,
    agent_id: Option<String>,
    call_type: Option<String>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Stamp(case) => serde_json::to_value(used_client_oauth_token(
                &case.request_data,
                case.request_route.as_deref(),
            ))
            .or_value_error(),
            Input::Patch(case) => patch(case),
            Input::Tags(case) => tags(case),
            Input::Eligibility(case) => eligibility(case),
        }
    }
}

fn patch(case: &PatchInput) -> HelperResult {
    let mut kwargs = case.request_data.clone();
    write_spend_metadata_to_kwargs(&mut kwargs, &case.metadata);
    serde_json::to_value(Json::Object(kwargs)).or_value_error()
}

fn tags(case: &TagsInput) -> HelperResult {
    let standard = case.standard_logging_object.clone().map(Json::Object);
    let metadata = Json::Object(case.metadata.clone());
    let tags = request_tags_for_cost_tracking(standard.as_ref(), Some(&metadata));
    serde_json::to_value(tags).or_value_error()
}

fn eligibility(case: &EligibilityInput) -> HelperResult {
    let spend_updates = if case.disable_spend_updates {
        SpendUpdates::Disabled
    } else {
        SpendUpdates::Allowed
    };
    let identity = TrackingIdentity {
        hashed_user_api_key: case.user_api_key.as_deref(),
        user_id: case.user_id.as_deref(),
        team_id: case.team_id.as_deref(),
        end_user_id: case.end_user_id.as_deref(),
        agent_id: case.agent_id.as_deref(),
    };
    serde_json::to_value(should_track_cost_callback(
        spend_updates,
        &identity,
        case.call_type.as_deref(),
    ))
    .or_value_error()
}
