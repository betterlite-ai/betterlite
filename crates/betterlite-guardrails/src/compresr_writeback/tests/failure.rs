use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use super::support::{TOOL_CONVERSATION, conversation, guardrail, reply, serve};
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::failure::GuardrailFailure;

const NO_RECOVERY: &str = "      enable_retrieval: false\n";
const OPEN: &str = "      enable_retrieval: false\n      unreachable_fallback: fail_open\n";
const SERVICE_ERROR: &str = "Compresr compression service returned an error";
const UNREADABLE: &str = "Compresr compression service returned an unreadable response";
const UNEXPECTED_SHAPE: &str = "Compresr compression service returned unexpected response shape";
const UNREACHABLE: &str = "Compresr compression service request failed";
const BATCH: &str = "Compresr batch response missing or mismatched 'results'";
const TWO_TOOLS: &str = r#"[{"role":"user","content":"find the answer"},{"role":"tool","tool_call_id":"c1","content":"long tool output one"},{"role":"tool","tool_call_id":"c2","content":"long tool output two"}]"#;
// Nothing listens on the discard port, so the connection is refused
const NOTHING_LISTENING: &str = "http://127.0.0.1:9";

#[rstest]
#[case::service_error(500, "secret upstream body", SERVICE_ERROR)]
#[case::unreadable(200, "secret upstream body", UNREADABLE)]
#[case::unexpected_shape(200, r#"{"data":"secret upstream body"}"#, UNEXPECTED_SHAPE)]
#[tokio::test]
async fn upstream_problem_fail_closed(
    #[case] status: u16,
    #[case] body: &str,
    #[case] message: &str,
) {
    let server = serve(status, body).await;
    let guardrail = guardrail(&server.base, NO_RECOVERY);
    let failure = guardrail
        .apply(
            conversation(TOOL_CONVERSATION, ""),
            &mut BTreeMap::new(),
            GuardrailInputType::Request,
            None,
        )
        .await
        .unwrap_err();
    let GuardrailFailure::HttpObject { status, detail } = &failure else {
        panic!("a failed compression is an HTTP error, got {failure:?}");
    };
    assert_eq!(*status, 502);
    assert_eq!(
        detail.get("error"),
        Some(&GuardrailValue::Text(message.to_owned()))
    );
    assert!(!failure.to_string().contains("secret upstream body"));
    assert!(!failure.to_string().contains("test-key"));
}

#[tokio::test]
async fn unreachable_service_fail_closed() {
    let guardrail = guardrail(NOTHING_LISTENING, NO_RECOVERY);
    let failure = guardrail
        .apply(
            conversation(TOOL_CONVERSATION, ""),
            &mut BTreeMap::new(),
            GuardrailInputType::Request,
            None,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&failure, GuardrailFailure::HttpObject { status: 502, detail }
            if detail.get("error") == Some(&GuardrailValue::Text(UNREACHABLE.to_owned())))
    );
}

#[tokio::test]
async fn a_mismatched_batch_fail_closed() {
    let server = serve(
        200,
        &reply(&[("one", 10, 4), ("two", 10, 4), ("three", 10, 4)]),
    )
    .await;
    let guardrail = guardrail(&server.base, NO_RECOVERY);
    let failure = guardrail
        .apply(
            conversation(TWO_TOOLS, ""),
            &mut BTreeMap::new(),
            GuardrailInputType::Request,
            None,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&failure, GuardrailFailure::HttpObject { status: 502, detail }
            if detail.get("error") == Some(&GuardrailValue::Text(BATCH.to_owned())))
    );
}

#[rstest]
#[case::service_error(500, "upstream body")]
#[case::unreadable(200, "upstream body")]
#[case::unexpected_shape(200, r#"{"data":"upstream body"}"#)]
#[tokio::test]
async fn upstream_problem_fail_open(#[case] status: u16, #[case] body: &str) {
    let server = serve(status, body).await;
    let guardrail = guardrail(&server.base, OPEN);
    let input = conversation(TOOL_CONVERSATION, "");
    let mut data = BTreeMap::new();
    let output = guardrail
        .apply(input.clone(), &mut data, GuardrailInputType::Request, None)
        .await
        .unwrap();
    assert_eq!(output, input);
    assert!(data.is_empty());
}

#[tokio::test]
async fn unreachable_service_fail_open() {
    let guardrail = guardrail(NOTHING_LISTENING, OPEN);
    let input = conversation(TOOL_CONVERSATION, "");
    let output = guardrail
        .apply(
            input.clone(),
            &mut BTreeMap::new(),
            GuardrailInputType::Request,
            None,
        )
        .await
        .unwrap();
    assert_eq!(output, input);
}
