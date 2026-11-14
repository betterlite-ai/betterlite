#![cfg(test)]

use betterlite_core::routes::CallType;
use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;

use crate::google_genai_guardrail::callbacks::{
    ApplyFuture, GenaiGuardrail, GuardrailCall, GuardrailInputs, InputType, process_request,
    process_response, process_stream,
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
    process_response(&mut response, None, &Object::new(), &mut recorder)
        .await
        .unwrap();

    let frames = vec![json(r#"{"candidates":[]}"#)];
    process_stream(&frames, None, &Object::new(), &mut recorder)
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
    process_response(&mut response, None, &Object::new(), &mut recorder)
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
        process_response(&mut response, None, &Object::new(), &mut recorder)
            .await
            .is_err()
    );
    let frames = vec![json(
        r#"{"candidates":[{"content":{"parts":[{"text":"x"}]}}]}"#,
    )];
    assert!(
        process_stream(&frames, None, &Object::new(), &mut recorder)
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
    process_stream(&frames, Some(&request_data), &Object::new(), &mut recorder)
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
}

#[test]
fn the_four_generate_content_call_types_are_advertised() {
    assert_eq!(
        crate::google_genai_guardrail::CALL_TYPES,
        [
            CallType::GenerateContent,
            CallType::AgenerateContent,
            CallType::GenerateContentStream,
            CallType::AgenerateContentStream,
        ]
    );
}
