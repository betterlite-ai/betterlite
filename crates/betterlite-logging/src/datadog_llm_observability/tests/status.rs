use rstest::rstest;

use crate::datadog_llm_observability::error::DatadogLlmObsError;
use crate::datadog_llm_observability::send::send_batch;
use crate::datadog_llm_observability::tests::support;

#[rstest]
#[case::accepted(202, None)]
#[case::ok(200, Some(200))]
#[case::created(201, Some(201))]
#[case::no_content(204, Some(204))]
#[case::client_error(400, Some(400))]
#[case::server_error(500, Some(500))]
#[tokio::test]
async fn datadog_llm_obs_http_status_validation_behavior(
    #[case] status: u16,
    #[case] expected: Option<u16>,
) {
    let environment = support::environment(&[("LITELLM_DD_AGENT_HOST", "agent.local")]);
    let settings =
        support::resolve(&support::agent_init(), &environment).expect("the settings resolve");
    let transport =
        support::ScriptedTransport::new(vec![support::Reply::body(status, "the intake refused")]);
    let outcome = send_batch(&settings, transport.as_ref(), vec![support::empty_span()]).await;
    match expected {
        None => assert!(outcome.is_ok()),
        Some(refused) => {
            let error = outcome.expect_err("the status is refused");
            assert!(matches!(
                error,
                DatadogLlmObsError::UnexpectedStatus { status: answered, .. } if answered == refused
            ));
            assert!(error.to_string().contains("the intake refused"));
        }
    }
}

#[tokio::test]
async fn datadog_llm_obs_transport_failures_are_reported() {
    let environment = support::environment(&[("LITELLM_DD_AGENT_HOST", "agent.local")]);
    let settings =
        support::resolve(&support::agent_init(), &environment).expect("the settings resolve");
    let transport = support::ScriptedTransport::new(vec![support::Reply::network()]);
    let error = send_batch(&settings, transport.as_ref(), vec![support::empty_span()])
        .await
        .expect_err("the request fails");
    assert!(matches!(error, DatadogLlmObsError::Transport(_)));
}
