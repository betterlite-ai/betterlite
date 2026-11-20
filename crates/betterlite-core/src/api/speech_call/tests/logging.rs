use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::speech::logging::SpeechLogRecord;

use super::super::call::SpeechCall;
use super::super::logging::SpeechCallLogging;
use super::support::*;

// the logging object the gateway passes, kept by the test that reads what speech writes into it
struct RecordingLogging {
    state: LoggingState,
    pre_calls: Vec<(String, String)>,
}

impl SpeechCallLogging for RecordingLogging {
    fn state(&mut self) -> &mut LoggingState {
        &mut self.state
    }

    fn pre_call(&mut self, record: SpeechLogRecord<'_>) -> Result<(), PrePostError> {
        self.pre_calls
            .push((record.input.to_owned(), record.api_base.to_owned()));
        Ok(())
    }
}

#[test]
fn the_call_record_and_the_pre_call_reach_the_logging_object() {
    let variables = [("OPENAI_API_KEY", "sk-env")];
    let harness = Harness::new(
        loaded_settings(SOURCE, &variables),
        RecordingTransport::ok("audio/mpeg", &[1]),
        &variables,
    );
    let native = SpeechCall {
        voice: named_voice("alloy"),
        response_format: Some(String::from("wav")),
        kwargs: object(r#"{"user": "u-1"}"#),
        ..base_call("openai/tts-1")
    };
    let mut logging = RecordingLogging {
        state: logging_state(),
        pre_calls: Vec::new(),
    };
    harness
        .run_logged(&native, &mut logging)
        .expect("the speech call answers");
    assert_eq!(logging.pre_calls.len(), 1);
    assert_eq!(logging.pre_calls[0].0, "hello");
    assert_eq!(logging.pre_calls[0].1, "https://api.openai.com/v1");
    assert_eq!(logging.state.model, "tts-1");
    assert_eq!(logging.state.user.as_deref(), Some("u-1"));
    assert_eq!(
        logging.state.optional_params.get("response_format"),
        Some(&Json::Text(String::from("wav")))
    );
    assert_eq!(
        logging.state.litellm_params.get("user"),
        Some(&Json::Text(String::from("u-1")))
    );
    assert_eq!(logging.state.custom_llm_provider.as_deref(), Some("openai"));
}
