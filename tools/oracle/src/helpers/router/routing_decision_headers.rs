use std::collections::BTreeMap;

use serde::Deserialize;

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

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let output = match input {
            Input::Complexity { request } => serde_json::to_value(
                betterlite_router::routing_decision_headers::complexity_headers(request),
            ),
            Input::Replace {
                existing,
                new_headers,
            } => serde_json::to_value(
                betterlite_router::routing_decision_headers::replace_complexity_headers(
                    existing,
                    new_headers,
                ),
            ),
            Input::Quality { headers, request } => {
                let mut headers = headers.clone();
                betterlite_router::routing_decision_headers::apply_quality_headers(
                    &mut headers,
                    request,
                );
                serde_json::to_value(headers)
            }
        };
        output.map_err(|error| {
            Box::new(crate::helpers::HelperError {
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
