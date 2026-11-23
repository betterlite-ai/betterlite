use crate::api::response_id_codecs::response::{build_response_id, decode_response_id};
use crate::api::responses_create::bridge::Created;

use super::support::{self, Outcome, ScriptedHandoff, ScriptedResolver, Token};

const MOCK_ID: &str = "resp_67ccd2bed1ec8190b14f964abc0542670bb6a6b452d3795b";
const METADATA: &str = r#"{"model":"openai/gpt-4o","input":"hello","kwargs":{"litellm_metadata":{"model_info":{"id":"deployment_a"}}}}"#;

pub(super) async fn result_identity() {
    completed().await;
    mock_shortcut().await;
    pending_once().await;
    encoded_untouched().await;
    untyped_passthrough().await;
}

async fn completed() {
    let created = run(Outcome::Completed(Some(support::stamped("resp_a")))).await;
    let expected = build_response_id(Some("openai"), Some("deployment_a"), "resp_a");
    assert_eq!(support::response_id(&created), expected);
    assert_eq!(
        support::hidden_provider(&created).as_deref(),
        Some("openai")
    );
    let decoded = decode_response_id(&support::response_id(&created));
    assert_eq!(
        decoded.custom_llm_provider,
        Some(Some(String::from("openai")))
    );
    assert_eq!(decoded.model_id, Some(Some(String::from("deployment_a"))));
    assert_eq!(decoded.response_id, Some(String::from("resp_a")));
}

// The mock response B1078 shortcuts still gets the deployment model_id and the provider stamp
async fn mock_shortcut() {
    let created = run(Outcome::Completed(Some(support::stamped(MOCK_ID)))).await;
    let expected = build_response_id(Some("openai"), Some("deployment_a"), MOCK_ID);
    assert_eq!(support::response_id(&created), expected);
    assert_eq!(
        support::hidden_provider(&created).as_deref(),
        Some("openai")
    );
}

// A coroutine answer is awaited once, then stamped the same way
async fn pending_once() {
    let handoff = ScriptedHandoff::new(Outcome::Pending(Some(support::stamped("resp_a"))));
    let resolver = ScriptedResolver::default();
    let call = support::call_from(METADATA);
    let created = support::drive(&resolver, None, &handoff, &call)
        .await
        .expect("the wrapper answers");
    assert_eq!(handoff.polls(), 2);
    assert_eq!(handoff.calls().len(), 1);
    let expected = build_response_id(Some("openai"), Some("deployment_a"), "resp_a");
    assert_eq!(support::response_id(&created), expected);
    assert_eq!(
        support::hidden_provider(&created).as_deref(),
        Some("openai")
    );
}

async fn encoded_untouched() {
    let encoded = build_response_id(Some("openai"), Some("deployment_a"), "resp_a");
    let created = run(Outcome::Completed(Some(support::stamped(&encoded)))).await;
    assert_eq!(support::response_id(&created), encoded);
    assert_eq!(
        support::hidden_provider(&created).as_deref(),
        Some("openai")
    );
}

// Only a typed response is rewritten; an iterator, a raw body and a bridged answer pass through
async fn untyped_passthrough() {
    match run(Outcome::Completed(Some(support::raw(r#"{"id":"resp_a"}"#)))).await {
        Created::Raw(value) => assert_eq!(value.get(), r#"{"id":"resp_a"}"#),
        _ => panic!("the raw body stays raw"),
    }
    match run(Outcome::Completed(Some(support::live()))).await {
        Created::Live(live) => assert_eq!(live.setup.model, "openai/gpt-4o"),
        _ => panic!("the live stream stays live"),
    }
    match run(Outcome::Completed(Some(support::synthetic()))).await {
        Created::Synthetic(synthetic) => {
            assert_eq!(synthetic.setup.custom_llm_provider, "openai");
        }
        _ => panic!("the synthetic stream stays synthetic"),
    }
}

async fn run(outcome: Outcome) -> Created<Token> {
    let handoff = ScriptedHandoff::new(outcome);
    let resolver = ScriptedResolver::default();
    let call = support::call_from(METADATA);
    support::drive(&resolver, None, &handoff, &call)
        .await
        .expect("the wrapper answers")
}
