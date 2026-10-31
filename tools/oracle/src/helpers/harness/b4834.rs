mod events;

use betterlite_harness::providers::claude_code::ClaudeCodeHarnessConfig;
use betterlite_harness::transformation::BaseCLIHarnessConfig as _;
use betterlite_harness::types::JsonObject;
use serde_json::json;
use std::collections::BTreeSet;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(serde::Deserialize)]
pub(crate) struct Input {
    stream: String,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let config = ClaudeCodeHarnessConfig;
        let mut state = config.create_stream_state();
        let mut started = BTreeSet::new();
        let mut output = Vec::new();
        for line in input.stream.lines().filter(|line| !line.is_empty()) {
            let event: JsonObject = serde_json::from_str(line).map_err(|error| {
                Box::new(HelperError {
                    class: "StreamParse".to_owned(),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: error.to_string(),
                })
            })?;
            for parsed in config.transform_stream_line(&event, &mut state) {
                output.push(events::encode(&parsed, &mut started)?);
            }
        }
        Ok(json!({
            "session_id": state.session_id,
            "final_text": state.final_text(),
            "structured_output": state.structured_output_json(),
            "events": output,
        }))
    }
}
