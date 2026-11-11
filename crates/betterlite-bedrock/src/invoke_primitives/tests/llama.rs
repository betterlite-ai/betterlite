use std::collections::BTreeMap;

use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::PromptFactory;

use crate::aws_endpoints::provider::BedrockInvokeProvider;
use crate::invoke_primitives::prompt::{
    ConversionError, CustomPromptEntry, PromptConversion, convert_messages_to_prompt,
};

use super::support::{clock, empty_factory, factory, messages, runtime};

const LLAMA3_INSTRUCT_MODEL: &str = "meta.llama3-8b-instruct-v1:0";
const LLAMA3_RENDERED: &str = "<|begin_of_text|><|start_header_id|>user<|end_header_id|>\n\nhi<|eot_id|><|start_header_id|>assistant<|end_header_id|>\n\n";

fn convert(
    model: &str,
    messages: &[Message],
    custom: &BTreeMap<String, CustomPromptEntry>,
    prompts: &PromptFactory,
) -> Result<String, ConversionError> {
    runtime()
        .block_on(convert_messages_to_prompt(PromptConversion {
            model,
            messages,
            provider: Some(BedrockInvokeProvider::Meta),
            custom_prompt_dict: custom,
            prompts,
            now: clock(),
        }))
        .map(|converted| converted.prompt)
}

pub(super) fn llama_local_template() {
    custom_bypass();
    local_assets();
    generic_fallback();
    empty_delimiters();
    earlier_branches_win();
}

fn custom_bypass() {
    let custom: BTreeMap<String, CustomPromptEntry> = serde_json::from_str(
        r#"{"meta.llama3-8b-instruct-v1:0":{"roles":{"user":{"pre_message":"[","post_message":"]"}}}}"#,
    )
    .unwrap();
    let exchange = messages(r#"[{"role":"user","content":"hi"}]"#);
    assert_eq!(
        convert(LLAMA3_INSTRUCT_MODEL, &exchange, &custom, &empty_factory()).unwrap(),
        "[hi]"
    );
}

fn local_assets() {
    let exchange = messages(r#"[{"role":"user","content":"hi"}]"#);
    assert_eq!(
        convert(
            LLAMA3_INSTRUCT_MODEL,
            &exchange,
            &BTreeMap::new(),
            &factory()
        )
        .unwrap(),
        LLAMA3_RENDERED
    );
    // missing local assets fail with LiteLLM's exact message instead of a download
    let error = convert(
        LLAMA3_INSTRUCT_MODEL,
        &exchange,
        &BTreeMap::new(),
        &empty_factory(),
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "No chat template found");
}

fn generic_fallback() {
    let conversation =
        messages(r#"[{"role":"system","content":"s"},{"role":"user","content":"u"}]"#);
    assert_eq!(
        convert(
            "meta.llama3-8b-v1:0",
            &conversation,
            &BTreeMap::new(),
            &empty_factory()
        )
        .unwrap(),
        "s u"
    );
}

fn empty_delimiters() {
    let template: BTreeMap<String, CustomPromptEntry> =
        serde_json::from_str(r#"{"meta.llama3-8b-instruct-v1:0":{"roles":{"user":{}}}}"#).unwrap();
    let exchange = messages(r#"[{"role":"user","content":"hi"}]"#);
    assert_eq!(
        convert(
            LLAMA3_INSTRUCT_MODEL,
            &exchange,
            &template,
            &empty_factory()
        )
        .unwrap(),
        "hi"
    );
}

fn earlier_branches_win() {
    let exchange = messages(r#"[{"role":"user","content":"hi"}]"#);
    // each earlier Bedrock branch renders without a template even when llama3 instruct also matches
    for (mixed, plain) in [
        (
            "amazon.titan-text-llama3-instruct",
            "amazon.titan-text-express-v1",
        ),
        ("anthropic.llama3-instruct", "anthropic.claude-v2"),
        ("mistral.llama3-instruct", "mistral.mixtral-8x7b-instruct"),
        ("llama2-chat-llama3-instruct", "meta.llama2-13b-chat"),
    ] {
        let rendered = convert(mixed, &exchange, &BTreeMap::new(), &empty_factory()).unwrap();
        assert_eq!(
            rendered,
            convert(plain, &exchange, &BTreeMap::new(), &empty_factory()).unwrap()
        );
    }
}
