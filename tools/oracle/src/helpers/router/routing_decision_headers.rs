use std::collections::BTreeMap;

use serde::Deserialize;

use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_router::routing_decision_headers;
use betterlite_router::routing_decision_headers::RoutingValue;

#[derive(Debug, Deserialize)]
#[serde(tag = "function")]
pub(crate) enum Input {
    #[serde(rename = "complexity")]
    Complexity { request: RoutingValue },
    #[serde(rename = "replace")]
    Replace {
        existing: BTreeMap<String, RoutingValue>,
        new_headers: BTreeMap<String, String>,
    },
    #[serde(rename = "quality")]
    Quality {
        headers: BTreeMap<String, RoutingValue>,
        request: RoutingValue,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let output = match input {
            Input::Complexity { request } => {
                serde_json::to_value(routing_decision_headers::complexity_headers(request))
            }
            Input::Replace {
                existing,
                new_headers,
            } => serde_json::to_value(routing_decision_headers::replace_complexity_headers(
                existing,
                new_headers,
            )),
            Input::Quality {
                headers: original_headers,
                request,
            } => {
                let mut headers = original_headers.clone();
                routing_decision_headers::apply_quality_headers(&mut headers, request);
                serde_json::to_value(headers)
            }
        };
        output.map_err(|error| {
            Box::new(HelperError {
                class: "SerializationError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}
