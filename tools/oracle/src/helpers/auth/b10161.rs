use betterlite_auth::jwt_claims::TypedValue;
use betterlite_auth::jwt_oauth_routing::{
    matches_routing_override, routing_selector_matches_claim,
};
// ast-grep-ignore: no-long-names
use betterlite_auth::jwt_oauth_routing::should_route_jwt_to_oauth2_override as route_to_oauth2;
use betterlite_config::sections::jwt_settings::records::{RoutingOverride, Selector};
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Selector {
        selector: Option<Selector>,
        claim: Option<ClaimValue>,
        split_space_delimited: bool,
    },
    Override {
        #[serde(rename = "override")]
        override_rule: RoutingOverride,
        claims: TypedValue,
    },
    Route {
        token: String,
        routing_overrides: Option<Vec<RoutingOverride>>,
    },
}

// a declared claim is a string or a list of strings; null and an absent key are the same None here
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum ClaimValue {
    One(String),
    Many(Vec<String>),
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        match input {
            Input::Selector {
                selector,
                claim,
                split_space_delimited,
            } => Ok(selector_answer(
                selector.as_ref(),
                claim.as_ref(),
                *split_space_delimited,
            )),
            Input::Override {
                override_rule,
                claims,
            } => Ok(json!({"matched": matches_routing_override(claims, override_rule)})),
            Input::Route {
                token,
                routing_overrides,
            } => Ok(json!({
                "matched": route_to_oauth2(token, routing_overrides.as_deref())
            })),
        }
    }
}

fn selector_answer(selector: Option<&Selector>, claim: Option<&ClaimValue>, split: bool) -> Value {
    let claims = match claim {
        Some(ClaimValue::One(text)) => json!({"iss": text}),
        Some(ClaimValue::Many(items)) => json!({"iss": items}),
        None => json!({"iss": null}),
    };
    let document = TypedValue { json: claims };
    let matched = routing_selector_matches_claim(selector, &document, "iss", split);
    json!({"matched": matched})
}
