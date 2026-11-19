use rstest::rstest;

use super::*;

#[tokio::test]
async fn deltas_accumulate_only_string_text_and_arguments() {
    let mut rig = Rig::new(Some(TransformMode::Direct));
    rig.process(TEXT_DELTA)
        .await
        .expect("the text delta is processed");
    rig.process(ARGS_DELTA)
        .await
        .expect("the argument delta is processed");
    rig.process(CUSTOM_DELTA)
        .await
        .expect("the custom tool delta is processed");
    rig.process(MCP_DELTA)
        .await
        .expect("the mcp delta is processed");
    rig.process(REFUSAL_DELTA)
        .await
        .expect("the refusal delta is processed");
    rig.process(NON_STRING_DELTA)
        .await
        .expect("the numeric delta is processed");
    rig.process(GENERIC_TEXT_DELTA)
        .await
        .expect("the generic delta is processed");
    assert_eq!(rig.state.generated_content, "hiZ");
    assert_eq!(rig.state.generated_tool_arguments, "{}XY");
}

fn estimation_rig() -> Rig<TransformMode> {
    let mut rig = Rig::new(Some(TransformMode::Direct));
    rig.tokenization = Some(Tokenization::new(30, 50));
    rig.calculator = Some(Calculator::missing());
    rig.state
        .request_data
        .insert("input".to_owned(), Json::Text("hello".to_owned()));
    rig.state
        .request_data
        .insert("model".to_owned(), Json::Text("gpt-4o".to_owned()));
    rig
}

#[tokio::test]
async fn terminal_estimation_receives_the_request_and_joined_generation() {
    let mut rig = estimation_rig();
    rig.process(TEXT_DELTA)
        .await
        .expect("the text delta is processed");
    rig.process(ARGS_DELTA)
        .await
        .expect("the argument delta is processed");
    rig.process(COMPLETED)
        .await
        .expect("the terminal is processed")
        .expect("the terminal is handed on");
    let calls = rig
        .tokenization
        .as_ref()
        .expect("the fixture tokenization exists")
        .calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, "gpt-4o");
    assert_eq!(calls[0].request_input, Json::Text("hello".to_owned()));
    assert_eq!(
        calls[0].request,
        Json::Object(rig.state.request_data.clone())
    );
    assert_eq!(calls[0].generated, "hi{}");
}

#[tokio::test]
async fn terminal_estimation_writes_the_usage_totals() {
    let mut rig = estimation_rig();
    rig.process(TEXT_DELTA)
        .await
        .expect("the text delta is processed");
    rig.process(ARGS_DELTA)
        .await
        .expect("the argument delta is processed");
    let event = rig
        .process(COMPLETED)
        .await
        .expect("the terminal is processed")
        .expect("the terminal is handed on");
    let json = event_json(&event);
    assert_eq!(
        number_text(at(&json, &["response", "usage", "input_tokens"])),
        "30"
    );
    assert_eq!(
        number_text(at(&json, &["response", "usage", "output_tokens"])),
        "50"
    );
    assert_eq!(
        number_text(at(&json, &["response", "usage", "total_tokens"])),
        "80"
    );
    assert_eq!(number_text(at(&json, &["response", "usage", "cost"])), "");
}

#[tokio::test]
async fn terminals_with_usage_skip_estimation() {
    let mut rig = Rig::new(Some(TransformMode::Direct));
    rig.tokenization = Some(Tokenization::new(30, 50));
    rig.calculator = Some(Calculator::missing());
    let completed = rig
        .process(COMPLETED_USAGE)
        .await
        .expect("the terminal is processed")
        .expect("the terminal is handed on");
    assert_eq!(
        number_text(at(
            &event_json(&completed),
            &["response", "usage", "total_tokens"]
        )),
        "16"
    );
    let failure = rig
        .process(FAILED_USAGE)
        .await
        .expect("the failure is processed")
        .expect("the failure is handed on");
    assert_eq!(
        number_text(at(
            &event_json(&failure),
            &["response", "usage", "input_tokens"]
        )),
        "7"
    );
    assert!(
        rig.tokenization
            .as_ref()
            .expect("the fixture tokenization exists")
            .calls()
            .is_empty()
    );
    await_pending_logging(&mut rig.callback).await;
}

#[tokio::test]
async fn bare_failures_never_estimate() {
    let mut rig = Rig::new(Some(TransformMode::Direct));
    rig.tokenization = Some(Tokenization::new(30, 50));
    let event = rig
        .process(FAILED_BARE)
        .await
        .expect("the bare failure is processed")
        .expect("the bare failure is handed on");
    assert_eq!(at(&event_json(&event), &["response", "usage"]), &Json::Null);
    assert!(
        rig.tokenization
            .as_ref()
            .expect("the fixture tokenization exists")
            .calls()
            .is_empty()
    );
    await_pending_logging(&mut rig.callback).await;
}

#[tokio::test]
async fn completed_terminal_is_stored_logged_and_returned_without_finishing() {
    let mut rig = Rig::new(Some(TransformMode::Direct));
    rig.tokenization = Some(Tokenization::new(30, 50));
    rig.calculator = Some(Calculator::charging("0.5"));
    rig.state
        .request_data
        .insert("input".to_owned(), Json::Text("hello".to_owned()));
    let event = rig
        .process(COMPLETED)
        .await
        .expect("the terminal is processed")
        .expect("the terminal is handed on");
    assert!(!rig.state.finished);
    assert!(rig.state.completed_response_logged);
    let stored = rig
        .state
        .completed_response
        .as_ref()
        .expect("the terminal is stored");
    assert_eq!(event_json(stored), event_json(&event));
    assert_eq!(rig.ports.success.calls.entries(), vec!["async", "sync"]);
    let payloads = rig.ports.success.payloads();
    assert_eq!(payloads.len(), 2);
    assert_eq!(
        number_text(at(&payloads[0], &["response", "usage", "cost"])),
        "0.5"
    );
    assert_eq!(
        number_text(at(&payloads[0], &["response", "usage", "total_tokens"])),
        "80"
    );
    assert_eq!(
        number_text(at(&event_json(&event), &["response", "usage", "cost"])),
        "0.5"
    );
    assert!(rig.ports.failure.calls.entries().is_empty());
}

#[tokio::test]
async fn incomplete_terminal_is_stored_and_logged() {
    let mut rig = Rig::new(Some(TransformMode::Direct));
    rig.tokenization = Some(Tokenization::new(30, 50));
    let event = rig
        .process(INCOMPLETE)
        .await
        .expect("the terminal is processed")
        .expect("the terminal is handed on");
    assert!(!rig.state.finished);
    assert!(rig.state.completed_response.is_some());
    assert_eq!(
        number_text(at(
            &event_json(&event),
            &["response", "usage", "total_tokens"]
        )),
        "80"
    );
    assert_eq!(rig.ports.success.calls.entries(), vec!["async", "sync"]);
    let calls = rig
        .tokenization
        .as_ref()
        .expect("the fixture tokenization exists")
        .calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].request_input,
        Json::Null,
        "an absent request input estimates as a null input"
    );
}

#[tokio::test]
async fn failed_terminal_logs_failure_with_usage_billing() {
    let mut rig = Rig::new(Some(TransformMode::Direct));
    rig.tokenization = Some(Tokenization::new(30, 50));
    rig.calculator = Some(Calculator::charging("0.5"));
    let event = rig
        .process(FAILED_USAGE)
        .await
        .expect("the failure is processed")
        .expect("the failure is handed on");
    await_pending_logging(&mut rig.callback).await;
    assert!(!rig.state.finished);
    assert!(rig.state.failure_handled);
    assert!(rig.state.completed_response.is_some());
    assert_eq!(rig.ports.failure.calls.entries(), vec!["async", "sync"]);
    assert!(rig.ports.success.calls.entries().is_empty());
    assert_eq!(
        number_text(at(&event_json(&event), &["response", "usage", "cost"])),
        "0.5"
    );
    let combined = rig
        .details
        .get("combined_usage_object")
        .expect("the failed usage is recorded");
    assert!(matches!(combined, Json::Object(_)));
    assert_eq!(
        number_text(
            rig.details
                .get("response_cost")
                .expect("the cost is recorded")
        ),
        "0.5"
    );
}

#[tokio::test]
async fn generic_terminal_is_billed_as_an_object() {
    let mut rig = Rig::new(Some(TransformMode::Direct));
    rig.tokenization = Some(Tokenization::new(30, 50));
    rig.calculator = Some(Calculator::charging("0.5"));
    let event = rig
        .process(GENERIC_COMPLETED)
        .await
        .expect("the generic terminal is processed")
        .expect("the generic terminal is handed on");
    let json = event_json(&event);
    assert_eq!(
        at(&json, &["response", "id"]),
        &Json::Text(build_response_id(Some("openai"), None, "resp_1"))
    );
    assert_eq!(
        number_text(at(&json, &["response", "usage", "total_tokens"])),
        "16"
    );
    assert_eq!(
        number_text(at(&json, &["response", "usage", "cost"])),
        "0.5"
    );
    assert_eq!(
        at(&json, &["response", "status"]),
        &Json::Null,
        "the unchecked construction fills the defaulted fields"
    );
    assert!(rig.state.completed_response.is_some());
    assert_eq!(rig.ports.success.calls.entries(), vec!["async", "sync"]);
}

#[rstest]
#[case::zero(Some("0"), COMPLETED, "", 1)]
#[case::missing(None, COMPLETED, "", 1)]
#[case::existing_usage(Some("0.5"), COMPLETED_COST, "0.25", 0)]
#[tokio::test]
async fn terminal_cost_is_stamped_only_when_the_calculator_answers_positive(
    #[case] answer: Option<&str>,
    #[case] payload: &str,
    #[case] stamped: &str,
    #[case] calls: usize,
) {
    let mut rig = Rig::new(Some(TransformMode::Direct));
    rig.tokenization = Some(Tokenization::new(30, 50));
    rig.calculator = Some(answer.map_or_else(Calculator::missing, Calculator::charging));
    let event = rig
        .process(payload)
        .await
        .expect("the terminal is processed")
        .expect("the terminal is handed on");
    let calculator = rig
        .calculator
        .as_ref()
        .expect("the fixture calculator exists");
    assert_eq!(calculator.calls(), calls);
    assert_eq!(
        number_text(at(&event_json(&event), &["response", "usage", "cost"])),
        stamped
    );
}

#[tokio::test]
async fn terminal_hook_failure_raises_after_failure_delivery() {
    let mut rig = Rig::with_ports(Ports::failing_cache(), Some(TransformMode::Direct));
    let Err(error) = rig.process(COMPLETED).await else {
        panic!("a failing terminal hook must surface");
    };
    assert!(matches!(&error, ProcessEventError::Hook(_)));
    assert_eq!(error.to_string(), "cache write failed");
    await_pending_logging(&mut rig.callback).await;
    assert!(rig.state.completed_response.is_some());
    assert!(rig.state.completed_response_logged);
    assert!(rig.state.failure_handled);
    assert_eq!(rig.ports.failure.calls.entries(), vec!["async", "sync"]);
    assert!(rig.ports.success.calls.entries().is_empty());
}

#[tokio::test]
async fn second_terminal_does_not_log_twice() {
    let mut rig = Rig::new(Some(TransformMode::Direct));
    rig.process(COMPLETED)
        .await
        .expect("the first terminal is processed");
    let second = rig
        .process(COMPLETED_USAGE)
        .await
        .expect("the second terminal is processed")
        .expect("the second terminal is handed on");
    assert_eq!(rig.ports.success.calls.entries(), vec!["async", "sync"]);
    let stored = rig
        .state
        .completed_response
        .as_ref()
        .expect("the terminal is stored");
    assert_eq!(
        number_text(at(
            &event_json(stored),
            &["response", "usage", "total_tokens"]
        )),
        "16"
    );
    assert_eq!(
        number_text(at(
            &event_json(&second),
            &["response", "usage", "total_tokens"]
        )),
        "16"
    );
}

#[tokio::test]
async fn generic_incomplete_terminal_keeps_its_branch() {
    let mut rig = Rig::new(Some(TransformMode::Direct));
    rig.tokenization = Some(Tokenization::new(30, 50));
    rig.calculator = Some(Calculator::charging("0.5"));
    let incomplete = rig
        .process(GENERIC_INCOMPLETE)
        .await
        .expect("the generic incomplete is processed")
        .expect("the generic incomplete is handed on");
    assert!(rig.state.completed_response.is_some());
    assert_eq!(rig.ports.success.calls.entries(), vec!["async", "sync"]);
    assert_eq!(
        number_text(at(
            &event_json(&incomplete),
            &["response", "usage", "total_tokens"]
        )),
        "80"
    );
}

#[tokio::test]
async fn generic_failed_terminal_keeps_its_branch() {
    let mut rig = Rig::new(Some(TransformMode::Direct));
    rig.tokenization = Some(Tokenization::new(30, 50));
    rig.calculator = Some(Calculator::charging("0.5"));
    let failed = rig
        .process(GENERIC_FAILED)
        .await
        .expect("the generic failure is processed")
        .expect("the generic failure is handed on");
    await_pending_logging(&mut rig.callback).await;
    assert!(rig.state.failure_handled);
    assert!(rig.state.completed_response.is_some());
    assert_eq!(rig.ports.failure.calls.entries(), vec!["async", "sync"]);
    assert!(rig.ports.success.calls.entries().is_empty());
    assert_eq!(
        at(&event_json(&failed), &["type"]),
        &Json::Text("response.failed".to_owned())
    );
}
