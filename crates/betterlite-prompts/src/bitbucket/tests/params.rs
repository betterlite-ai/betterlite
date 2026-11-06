use betterlite_prompts::bitbucket::manager::BitBucketPromptManager;
use tokio::sync::OnceCell;

use betterlite_models::decisions::Json as ContractJson;
use betterlite_models::prompt_contracts::PromptParams;

use crate::support;

fn contract_number(value: Option<&ContractJson>) -> Option<f64> {
    let present = value?;
    match present {
        ContractJson::Number(number) => number.as_f64(),
        ContractJson::Null
        | ContractJson::Bool(_)
        | ContractJson::Text(_)
        | ContractJson::List(_)
        | ContractJson::Object(_) => None,
    }
}

async fn compiled_params(template: &str) -> PromptParams {
    let server = support::Server::start(&[("greeting.prompt", 200, template)]);
    let manager = BitBucketPromptManager {
        bitbucket_config: support::config(&server.base_url),
        prompt_id: None,
        transport: support::transport(),
        template_manager: OnceCell::const_new(),
    };
    let client = manager
        .compile_prompt_helper(Some("greeting"), None, None, None)
        .await
        .expect("compiled prompt");
    client.prompt_optional_params.expect("optional params")
}

#[tokio::test]
async fn compile_projects_the_five_optional_params_and_keeps_null_values() {
    let params = compiled_params(
        "---\nmodel: gpt-4\ntemperature: 0.5\nmax_tokens: 100\ntop_p: 0.9\n\
         frequency_penalty: 0.1\npresence_penalty:\n---\nUser: hi",
    )
    .await;

    assert_eq!(params.len(), 5);
    assert_eq!(contract_number(params.get("temperature")), Some(0.5_f64));
    assert_eq!(contract_number(params.get("max_tokens")), Some(100.0_f64));
    assert_eq!(contract_number(params.get("top_p")), Some(0.9_f64));
    assert_eq!(
        contract_number(params.get("frequency_penalty")),
        Some(0.1_f64)
    );
    assert!(matches!(
        params.get("presence_penalty"),
        Some(ContractJson::Null)
    ));
}

#[tokio::test]
async fn compile_carries_temperature_and_max_tokens_as_nulls_when_absent() {
    let params = compiled_params("---\nmodel: gpt-4\n---\nUser: hi").await;

    assert_eq!(
        params.len(),
        2,
        "LiteLLM's metadata always carries temperature and max_tokens"
    );
    assert!(matches!(
        params.get("temperature"),
        Some(ContractJson::Null)
    ));
    assert!(matches!(params.get("max_tokens"), Some(ContractJson::Null)));
}
