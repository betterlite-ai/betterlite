#![cfg(test)]

use super::*;
use crate::models::finding::Role;
use rstest::rstest;
use std::fs;
use std::path::PathBuf;

#[test]
fn records_have_the_expected_shapes() {
    let observation: Observation =
        serde_json::from_str(r#"{"check_id":"c","summary":"ok"}"#).unwrap();
    assert_eq!(observation.kind, ObservationKind::Issue);
    let decision: Decision = serde_json::from_str(r#"{"action":"read"}"#).unwrap();
    assert_eq!(decision.page, 0);
}

#[test]
fn evidence_does_not_cross_omission_marker() {
    let part = TracePart {
        execution_id: String::from("e"),
        span_id: String::from("s"),
        parent_span_id: String::new(),
        name: String::from("n"),
        kind: String::from("k"),
        content: String::from("left\n[... content omitted ...]\nright"),
        truncated: false,
    };
    let evidence = Evidence {
        execution_id: String::from("e"),
        span_id: String::from("s"),
        quote: String::from("leftright"),
        role: Role::default(),
    };
    assert!(!evidence_valid(&evidence, &[part]));
}

#[test]
fn partition_rejects_zero_limit() {
    partition_items(&[1_i32], |_| 1_usize, 0).unwrap_err();
}

#[test]
fn content_partitions_account_for_serialized_bytes() {
    let part: TracePart = serde_json::from_str(
        r#"{"execution_id":"e","span_id":"s","name":"n","kind":"k","content":"\u00e9\\\""}"#,
    )
    .unwrap();
    let width = serde_json::to_vec(&part).unwrap().len() + 20;
    let parts = vec![part.clone(), part.clone()];
    assert_eq!(
        partition_content(&parts, width).unwrap(),
        vec![vec![part.clone()], vec![part]]
    );
    assert_eq!(
        partition_content(&parts, width * 2).unwrap(),
        vec![parts.clone()]
    );
    partition_content(&parts, width - 1).unwrap_err();
    assert!(partition_content(&[], width).unwrap().is_empty());
}

#[test]
fn default_concurrency_is_eight() {
    assert_eq!(ANALYSIS_CONCURRENCY, 8);
}

#[rstest]
fn replays_the_structured_response_corpus(
    #[files("fuzz/corpus/structured_response/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    drop(Decision::model_validate_json(&bytes));
    drop(FinalDecision::model_validate_json(&bytes));
    drop(Examined::model_validate_json(&bytes));
}

#[test]
fn review_inherits_cannot_assess() {
    let review = decode::<TraceReview>(br#"{"cannot_assess":true}"#).unwrap();
    assert!(review.cannot_assess);
    assert!(!TraceReview::default().cannot_assess);
}

#[rstest]
#[case::json("not json")]
#[case::schema(r#"{"action":"read","page":-1}"#)]
#[case::semantic(r#"{"action":"read"}"#)]
#[tokio::test]
async fn repairs_once(#[case] first: &str) {
    use crate::models::messages::Purpose;
    use rust_decimal::Decimal;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = AtomicUsize::new(0);
    let request = ModelRequest {
        prompt: String::from("original evidence"),
        purpose: Purpose::Extract,
    };
    let model = |sent: ModelRequest| {
        let attempt = calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(sent.purpose, Purpose::Extract);
        assert!(sent.prompt.starts_with("original evidence"));
        if attempt == 1 {
            assert!(sent.prompt.contains("validation errors:"));
        }
        async move {
            Ok(ModelResult {
                content: String::from(if attempt == 0 {
                    first
                } else {
                    r#"{"action":"submit"}"#
                }),
                cost: Decimal::ZERO,
            })
        }
    };
    let decision: Decision = structured_response(request, model, |parsed: &Decision| {
        (parsed.action != DecisionAction::Submit).then(|| String::from("must submit"))
    })
    .await
    .unwrap();
    assert_eq!(decision.action, DecisionAction::Submit);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[rstest]
#[case::negative(br#"{"action":"read","offset":-1}"#, false)]
#[case::zero(br#"{"action":"read","offset":0}"#, true)]
#[case::unknown(br#"{"action":"bad"}"#, false)]
fn decision_decoder_validates(#[case] bytes: &[u8], #[case] valid: bool) {
    assert_eq!(Decision::model_validate_json(bytes).is_ok(), valid);
}

#[rstest]
#[case::finished(None)]
#[case::empty(Some(""))]
#[case::repeated(Some("next"))]
#[tokio::test]
async fn read_summary_preserves_final_page(#[case] last: Option<&'static str>) {
    use crate::trace_store::trace_store;
    let execution = Examined::model_validate_json(include_bytes!(
        "../../fuzz/corpus/structured_response/examined"
    ))
    .unwrap()
    .execution;
    let reply = execution.clone();
    let read: ReadContent = Arc::new(move |id, cursor, offset| {
        assert_eq!(id, "e");
        assert_eq!(offset, 0);
        let mut result = reply.clone();
        result.name = String::from("latest");
        Box::pin(async move {
            Ok(ExecutionContent {
                execution: result,
                parts: Vec::new(),
                partial: cursor.is_empty(),
                next_cursor: if cursor.is_empty() {
                    Some(String::from("next"))
                } else {
                    last.map(String::from)
                },
            })
        })
    });
    let mut store = trace_store().unwrap();
    let summary = read_execution(execution, &read, &mut store).await.unwrap();
    assert_eq!(summary.execution.name, "latest");
    assert_eq!(summary.next_cursor.as_deref(), last);
    assert!(summary.parts.is_empty());
    assert!(summary.partial);
}

#[rstest]
#[case::valid(r#"{"action":"inconclusive"}"#, 1)]
#[case::invalid("{", 2)]
#[tokio::test]
async fn final_response_limits_attempts(#[case] content: &str, #[case] attempts: usize) {
    use crate::models::messages::Purpose;
    use rust_decimal::Decimal;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = AtomicUsize::new(0);
    let model = |_| {
        calls.fetch_add(1, Ordering::SeqCst);
        async {
            Ok(ModelResult {
                content: String::from(content),
                cost: Decimal::ZERO,
            })
        }
    };
    let result = structured_response::<FinalDecision, _, _, _>(
        ModelRequest {
            prompt: String::from("evidence"),
            purpose: Purpose::Investigate,
        },
        model,
        |_| None,
    )
    .await;
    assert_eq!(result.is_ok(), attempts == 1);
    assert_eq!(calls.load(Ordering::SeqCst), attempts);
}
