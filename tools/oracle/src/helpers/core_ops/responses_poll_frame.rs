use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::responses_poll_stream::frame::{PollFrame, decode_poll_stream_frame};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
    frames: Vec<String>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let results = input
            .frames
            .iter()
            .map(|frame| frame_result(frame))
            .collect::<Result<Vec<Value>, Box<HelperError>>>()?;
        Ok(json!({ "results": results }))
    }
}

fn frame_result(frame: &str) -> Result<Value, Box<HelperError>> {
    let decoded =
        decode_poll_stream_frame(frame.as_bytes(), ApiLimits::default()).or_value_error()?;
    let (kind, event) = match decoded {
        PollFrame::NoData => ("no_data", Json::Null),
        PollFrame::Done => ("done", Json::Null),
        PollFrame::Event(event) => ("event", event),
        PollFrame::Malformed => ("malformed", Json::Null),
    };
    Ok(json!({ "kind": kind, "event": event }))
}
