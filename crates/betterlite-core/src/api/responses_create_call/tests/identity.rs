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

fn assert_identity(created: &Created<Token>, expected: &str) {
    let typed = support::typed(created);
    assert_eq!(typed.response.id, expected);
    assert_eq!(
        typed.hidden.get("custom_llm_provider"),
        Some(&support::json("\"openai\""))
    );
}

async fn completed() {
    let created = run(Outcome::Completed(Some(support::stamped("resp_a")))).await;
    let expected = build_response_id(Some("openai"), Some("deployment_a"), "resp_a");
    assert_identity(&created, &expected);
    let decoded = decode_response_id(&support::typed(&created).response.id);
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
    assert_identity(&created, &expected);
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
    assert_identity(&created, &expected);
}

async fn encoded_untouched() {
    let encoded = build_response_id(Some("openai"), Some("deployment_a"), "resp_a");
    let created = run(Outcome::Completed(Some(support::stamped(&encoded)))).await;
    assert_identity(&created, &encoded);
}

// Only a typed response is rewritten; an iterator, a raw body and a bridged answer pass through
async fn untyped_passthrough() {
    match run(Outcome::Completed(Some(support::raw(r#"{"id":"resp_a"}"#)))).await {
        Created::Raw(value) => assert_eq!(value.get(), r#"{"id":"resp_a"}"#),
        Created::Responses(_) | Created::Live(_) | Created::Synthetic(_) | Created::Chat(_) => {
            panic!("the raw body stays raw")
        }
    }
    match run(Outcome::Completed(Some(support::live()))).await {
        Created::Live(live) => assert_eq!(live.setup.model, "openai/gpt-4o"),
        Created::Responses(_) | Created::Raw(_) | Created::Synthetic(_) | Created::Chat(_) => {
            panic!("the live stream stays live")
        }
    }
    match run(Outcome::Completed(Some(support::synthetic()))).await {
        Created::Synthetic(synthetic) => {
            assert_eq!(synthetic.setup.custom_llm_provider, "openai");
        }
        Created::Responses(_) | Created::Raw(_) | Created::Live(_) | Created::Chat(_) => {
            panic!("the synthetic stream stays synthetic")
        }
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
