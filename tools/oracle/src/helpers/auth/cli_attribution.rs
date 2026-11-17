mod tests;

use std::collections::BTreeMap;

use betterlite_auth::cli_attribution::build::build_attribution_metadata;
use betterlite_auth::cli_attribution::claim_map::decode_cli_claim_map;
use betterlite_auth::cli_attribution::metadata::{
    flatten_metadata_for_poll, merge_metadata, poll_attribution_metadata,
};
use betterlite_auth::sso_types::identity::OpenIdIdentity;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Build {
        claim_map: String,
        identity: OpenIdIdentity,
    },
    Merge {
        existing_metadata: BTreeMap<String, JsonData>,
        attribution_metadata: BTreeMap<String, JsonData>,
    },
    Flatten {
        attribution_metadata: BTreeMap<String, JsonData>,
    },
    Poll {
        session: BTreeMap<String, JsonData>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Build {
                claim_map,
                identity,
            } => build(claim_map, identity),
            Input::Merge {
                existing_metadata,
                attribution_metadata,
            } => merge(existing_metadata, attribution_metadata),
            Input::Flatten {
                attribution_metadata,
            } => flatten(attribution_metadata),
            Input::Poll { session } => poll(session),
        }
    }
}

fn build(claim_map: &str, identity: &OpenIdIdentity) -> HelperResult {
    let metadata = build_attribution_metadata(identity, &decode_cli_claim_map(claim_map))
        .map_err(|error| value_error(&error.to_string()))?;
    encode(metadata)
}

fn merge(
    existing_metadata: &BTreeMap<String, JsonData>,
    attribution_metadata: &BTreeMap<String, JsonData>,
) -> HelperResult {
    let mut merged = existing_metadata.clone();
    merge_metadata(&mut merged, attribution_metadata);
    encode(merged)
}

fn flatten(attribution_metadata: &BTreeMap<String, JsonData>) -> HelperResult {
    encode(flatten_metadata_for_poll(attribution_metadata))
}

fn poll(session: &BTreeMap<String, JsonData>) -> HelperResult {
    encode(poll_attribution_metadata(session))
}

fn encode(metadata: BTreeMap<String, JsonData>) -> HelperResult {
    serde_json::to_value(metadata).map_err(|error| value_error(&error.to_string()))
}

fn value_error(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
