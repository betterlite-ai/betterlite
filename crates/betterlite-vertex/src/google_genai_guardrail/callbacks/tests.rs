#![cfg(test)]

use betterlite_core::routes::CallType;
use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;

use crate::google_genai_guardrail::CALL_TYPES;
use crate::google_genai_guardrail::callbacks::{
    ApplyFuture, GenaiGuardrail, GuardedOutput, GuardrailCall, GuardrailInputs, InputType,
    process_output, process_request,
};

#[derive(Debug, thiserror::Error)]
#[error("the recording guardrail blocks")]
struct Blocked;

#[derive(Debug, Clone)]
struct Recorded {
    texts: Vec<String>,
    model: Option<String>,
    input_type: InputType,
    request_data: Object,
}

struct Recorder {
    replacement: Option<Vec<String>>,
    blocked: bool,
    calls: Vec<Recorded>,
}

impl GenaiGuardrail for Recorder {
    type Failure = Blocked;

    fn apply_guardrail<'call>(
        &'call mut self,
        inputs: GuardrailInputs,
        call: GuardrailCall<'call>,
    ) -> ApplyFuture<'call, Self::Failure> {
        self.calls.push(Recorded {
            texts: inputs.texts.clone(),
            model: inputs.model.clone(),
            input_type: call.input_type,
            request_data: call.request_data.clone(),
        });
        let blocked = self.blocked;
        let replacement = match self.replacement.clone() {
            Some(replacement) => replacement,
            None => inputs.texts,
        };
        Box::pin(async move {
            if blocked {
                return Err(Blocked);
            }
            Ok(Some(replacement))
        })
    }
}

fn object(source: &str) -> Object {
    serde_json::from_str(source).unwrap()
}

fn json(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

#[tokio::test]
async fn empty_skip() {
    let mut recorder = Recorder {
        replacement: None,
        blocked: true,
        calls: Vec::new(),
    };
    let mut data = object(r#"{"contents":[{"parts":[{"inlineData":{}}]}]}"#);
    let original = Json::Object(data.clone());
    process_request(&mut data, &mut recorder).await.unwrap();
    assert_eq!(Json::Object(data), original);

    let mut response = object(r#"{"candidates":[]}"#);
    process_output(
        GuardedOutput::Response(&mut response),
        None,
        &Object::new(),
        &mut recorder,
    )
    .await
    .unwrap();

    let frames = vec![json(r#"{"candidates":[]}"#)];
    process_output(
        GuardedOutput::Frames(frames.as_slice()),
        None,
        &Object::new(),
        &mut recorder,
    )
    .await
    .unwrap();
    assert!(recorder.calls.is_empty());
}

#[rstest]
#[case::string(
    r#"{"model":"gemini-2.5-flash","contents":[{"parts":[{"text":"x"}]}]}"#,
    Some("gemini-2.5-flash")
)]
#[case::empty_string(r#"{"model":"","contents":[{"parts":[{"text":"x"}]}]}"#, Some(""))]
#[case::number(r#"{"model":7,"contents":[{"parts":[{"text":"x"}]}]}"#, None)]
#[case::null(r#"{"model":null,"contents":[{"parts":[{"text":"x"}]}]}"#, None)]
#[case::absent(r#"{"contents":[{"parts":[{"text":"x"}]}]}"#, None)]
#[tokio::test]
async fn model_string_only(#[case] source: &str, #[case] expected: Option<&str>) {
    let mut recorder = Recorder {
        replacement: None,
        blocked: false,
        calls: Vec::new(),
    };
    let mut data = object(source);
    process_request(&mut data, &mut recorder).await.unwrap();
    assert_eq!(recorder.calls.len(), 1);
    assert_eq!(recorder.calls[0].model.as_deref(), expected);
    assert_eq!(recorder.calls[0].input_type, InputType::Request);

    let mut response = object(r#"{"candidates":[{"content":{"parts":[{"text":"x"}]}}]}"#);
    process_output(
        GuardedOutput::Response(&mut response),
        Some(&object(source)),
        &Object::new(),
        &mut recorder,
    )
    .await
    .unwrap();
    assert_eq!(recorder.calls.len(), 2);
    assert_eq!(recorder.calls[1].model.as_deref(), expected);
    assert_eq!(recorder.calls[1].input_type, InputType::Response);
}

#[tokio::test]
async fn the_caller_request_data_reaches_the_guardrail() {
    let mut recorder = Recorder {
        replacement: None,
        blocked: false,
        calls: Vec::new(),
    };
    let mut data = object(r#"{"model":"gemini","contents":[{"parts":[{"text":"raw"}]}]}"#);
    process_request(&mut data, &mut recorder).await.unwrap();
    assert_eq!(
        Json::Object(recorder.calls[0].request_data.clone()),
        json(r#"{"model":"gemini","contents":[{"parts":[{"text":"raw"}]}]}"#)
    );
}

#[tokio::test]
async fn a_supplied_response_key_is_kept_and_metadata_is_added() {
    let mut recorder = Recorder {
        replacement: None,
        blocked: false,
        calls: Vec::new(),
    };
    let mut response = object(r#"{"candidates":[{"content":{"parts":[{"text":"raw"}]}}]}"#);
    let request_data = object(r#"{"model":"gemini-2.5-flash","response":null}"#);
    let metadata = object(r#"{"user_api_key_user_id":"u1"}"#);
    process_output(
        GuardedOutput::Response(&mut response),
        Some(&request_data),
        &metadata,
        &mut recorder,
    )
    .await
    .unwrap();
    assert_eq!(
        Json::Object(recorder.calls[0].request_data.clone()),
        json(
            r#"{"model":"gemini-2.5-flash","response":null,"litellm_metadata":{"user_api_key_user_id":"u1"}}"#
        )
    );
}

#[tokio::test]
async fn an_absent_response_key_gets_the_pre_write_response() {
    let mut recorder = Recorder {
        replacement: None,
        blocked: false,
        calls: Vec::new(),
    };
    let mut response = object(r#"{"candidates":[{"content":{"parts":[{"text":"raw"}]}}]}"#);
    let original = Json::Object(response.clone());
    process_output(
        GuardedOutput::Response(&mut response),
        Some(&object(r#"{"model":"gemini"}"#)),
        &Object::new(),
        &mut recorder,
    )
    .await
    .unwrap();
    assert_eq!(
        recorder.calls[0].request_data.get("response"),
        Some(&original)
    );
}

#[tokio::test]
async fn request_and_response_replacements_are_written_back() {
    let mut recorder = Recorder {
        replacement: Some(vec!["masked".to_owned()]),
        blocked: false,
        calls: Vec::new(),
    };
    let mut data = object(r#"{"model":"gemini","contents":[{"parts":[{"text":"raw"}]}]}"#);
    process_request(&mut data, &mut recorder).await.unwrap();
    assert_eq!(
        Json::Object(data),
        json(r#"{"model":"gemini","contents":[{"parts":[{"text":"masked"}]}]}"#)
    );

    let mut response = object(r#"{"candidates":[{"content":{"parts":[{"text":"raw"}]}}]}"#);
    process_output(
        GuardedOutput::Response(&mut response),
        None,
        &Object::new(),
        &mut recorder,
    )
    .await
    .unwrap();
    assert_eq!(
        Json::Object(response),
        json(r#"{"candidates":[{"content":{"parts":[{"text":"masked"}]}}]}"#)
    );
    assert_eq!(recorder.calls[1].input_type, InputType::Response);
}

#[tokio::test]
async fn block_propagates() {
    let mut recorder = Recorder {
        replacement: None,
        blocked: true,
        calls: Vec::new(),
    };
    let mut data = object(r#"{"contents":[{"parts":[{"text":"x"}]}]}"#);
    assert!(process_request(&mut data, &mut recorder).await.is_err());
    let mut response = object(r#"{"candidates":[{"content":{"parts":[{"text":"x"}]}}]}"#);
    assert!(
        process_output(
            GuardedOutput::Response(&mut response),
            None,
            &Object::new(),
            &mut recorder,
        )
        .await
        .is_err()
    );
    let frames = vec![json(
        r#"{"candidates":[{"content":{"parts":[{"text":"x"}]}}]}"#,
    )];
    assert!(
        process_output(
            GuardedOutput::Frames(frames.as_slice()),
            None,
            &Object::new(),
            &mut recorder,
        )
        .await
        .is_err()
    );
    assert_eq!(recorder.calls.len(), 3);
}

#[tokio::test]
async fn stream_replacement_ignored() {
    let mut recorder = Recorder {
        replacement: Some(vec!["changed".to_owned()]),
        blocked: false,
        calls: Vec::new(),
    };
    let frames = vec![
        json(r#"{"candidates":[{"content":{"parts":[{"text":"one"}]}}]}"#),
        Json::Text(
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"two\"}]}}]}\n\n"
                .to_owned(),
        ),
    ];
    let original = frames.clone();
    let request_data = object(r#"{"model":"gemini"}"#);
    let metadata = object(r#"{"user_api_key_user_id":"u1"}"#);
    process_output(
        GuardedOutput::Frames(frames.as_slice()),
        Some(&request_data),
        &metadata,
        &mut recorder,
    )
    .await
    .unwrap();
    assert_eq!(frames, original);
    assert_eq!(recorder.calls.len(), 1);
    let call = &recorder.calls[0];
    assert_eq!(call.texts, vec!["onetwo".to_owned()]);
    assert_eq!(call.model, None);
    assert_eq!(call.input_type, InputType::Response);
    assert_eq!(
        call.request_data.get("responses_so_far"),
        Some(&Json::List(original))
    );
    assert_eq!(
        call.request_data.get("litellm_metadata"),
        Some(&json(r#"{"user_api_key_user_id":"u1"}"#))
    );
}

#[test]
fn the_four_generate_content_call_types_are_advertised() {
    assert_eq!(
        CALL_TYPES,
        [
            CallType::GenerateContent,
            CallType::AgenerateContent,
            CallType::GenerateContentStream,
            CallType::AgenerateContentStream,
        ]
    );
}
