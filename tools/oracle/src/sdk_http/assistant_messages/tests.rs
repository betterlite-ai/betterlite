#![cfg(test)]

use betterlite_prompts::arguments::json::Json;

use crate::sdk_http::OracleLimits;
use crate::sdk_http::call_kind::CallKind as _;

use super::{Call, Mode, Operation, Run, Settings};

fn append_run() -> Run {
    Run {
        mode: Mode::Sync,
        operation: Operation::Append,
        provider: String::from("openai"),
        thread_id: String::from("thread_a"),
        role: Some(String::from("user")),
        content: Some(Json::Text(String::from("hello"))),
        attachments: None,
        metadata: None,
        params: None,
        settings: Settings::default(),
        drop_params: false,
        client: None,
    }
}

// LiteLLM accepts client= and hands it to the provider call, which betterlite cannot express (docs/deviations/B1057.yaml)
#[test]
fn validate_refuses_a_supplied_client() {
    Call {
        runs: vec![append_run()],
    }
    .validate(&OracleLimits::default())
    .expect("a client-less append run is valid");
    let mut supplied = append_run();
    supplied.client = Some(Json::Text(String::from("client")));
    let error = Call {
        runs: vec![supplied],
    }
    .validate(&OracleLimits::default())
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid sdk_http case: a message run needs a provider and a thread_id and no client"
    );
}
