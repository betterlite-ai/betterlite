use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_prompts::builtin::Message;
use betterlite_prompts::error::FactoryError;
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::hub::{
    ChatTemplate, ChatTemplateFile, Hub, HubMap, TokenizerConfig, TokenizerDocument,
    builtin_entries,
};
use betterlite_prompts::jinja::TokenValue;

use crate::aws_endpoints::provider::BedrockInvokeProvider;
use crate::invoke_primitives::prompt::{
    ConversionError, CustomPromptEntry, PromptConversion, convert_messages_to_prompt,
};

use super::support::{clock, empty_factory, factory, messages, runtime};

const LLAMA3_INSTRUCT_MODEL: &str = "meta.llama3-8b-instruct-v1:0";
const LLAMA3_HF_MODEL: &str = "meta-llama/Meta-Llama-3-8B-Instruct";
const LLAMA3_RENDERED: &str = "<|begin_of_text|><|start_header_id|>user<|end_header_id|>\n\nhi<|eot_id|><|start_header_id|>assistant<|end_header_id|>\n\n";
const MAP_TEMPLATE: &str = "{{ bos_token }}{% for message in messages %}[{{ message['role'] }}]{{ message['content'] }}{% endfor %}{{ eos_token }}";
const FILE_TEMPLATE: &str = "{{ bos_token }}{% for message in messages %}<{{ message['content'] }}>{% endfor %}{{ eos_token }}";

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
    // an empty factory fails because both fetches answer failure, not because a lookup refuses to download
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

fn llama_document(chat_template: ChatTemplate, bos: &str, eos: &str) -> TokenizerDocument {
    TokenizerDocument {
        chat_template,
        bos_token: Some(TokenValue::Text(bos.to_owned())),
        eos_token: Some(TokenValue::Text(eos.to_owned())),
    }
}

fn mapped_factory(config: Option<TokenizerDocument>, template: Option<&str>) -> PromptFactory {
    let configs = BTreeMap::from([(
        LLAMA3_HF_MODEL.to_owned(),
        config.map_or(TokenizerConfig::Failure, |document| {
            TokenizerConfig::Success(Some(document))
        }),
    )]);
    let templates = BTreeMap::from([(
        LLAMA3_HF_MODEL.to_owned(),
        template.map_or(ChatTemplateFile::Failure, |text| {
            ChatTemplateFile::Success(text.to_owned())
        }),
    )]);
    PromptFactory {
        cache: Mutex::new(BTreeMap::new()),
        hub: Hub::Map(HubMap { configs, templates }),
    }
}

#[test]
fn fetch_renders_map_template() {
    let exchange = messages(r#"[{"role":"user","content":"hi"}]"#);
    let prompts = mapped_factory(
        Some(llama_document(
            ChatTemplate::Text(MAP_TEMPLATE.to_owned()),
            "<|begin_of_text|>",
            "<|end_of_text|>",
        )),
        None,
    );
    assert_eq!(
        convert(LLAMA3_INSTRUCT_MODEL, &exchange, &BTreeMap::new(), &prompts).unwrap(),
        "<|begin_of_text|>[user]hi<|end_of_text|>"
    );
    // llama4 instruct reaches the same branch and the same fetched template
    assert_eq!(
        convert(
            "meta.llama4-17b-instruct-v1:0",
            &exchange,
            &BTreeMap::new(),
            &prompts
        )
        .unwrap(),
        "<|begin_of_text|>[user]hi<|end_of_text|>"
    );
}

#[test]
fn fetch_falls_back_to_file() {
    let exchange = messages(r#"[{"role":"user","content":"hi"}]"#);
    let prompts = mapped_factory(
        Some(llama_document(ChatTemplate::Absent, "<s>", "</s>")),
        Some(FILE_TEMPLATE),
    );
    assert_eq!(
        convert(LLAMA3_INSTRUCT_MODEL, &exchange, &BTreeMap::new(), &prompts).unwrap(),
        "<s><hi></s>"
    );
}

#[test]
fn fetch_refusal_is_no_template() {
    let exchange = messages(r#"[{"role":"user","content":"hi"}]"#);
    let prompts = PromptFactory {
        cache: Mutex::new(BTreeMap::new()),
        hub: Hub::Map(HubMap {
            configs: BTreeMap::new(),
            templates: BTreeMap::new(),
        }),
    };
    let error = convert(LLAMA3_INSTRUCT_MODEL, &exchange, &BTreeMap::new(), &prompts).unwrap_err();
    assert_eq!(error.to_string(), "No chat template found");
    assert!(matches!(
        error,
        ConversionError::Factory(FactoryError::NoChatTemplate)
    ));
}

#[test]
fn fetch_cache_hit_skips_hub() {
    let exchange = messages(r#"[{"role":"user","content":"hi"}]"#);
    let prompts = PromptFactory {
        cache: Mutex::new(builtin_entries()),
        hub: Hub::Map(HubMap {
            configs: BTreeMap::from([(
                LLAMA3_HF_MODEL.to_owned(),
                TokenizerConfig::Success(Some(llama_document(
                    ChatTemplate::Text("{{ messages[0]['content'] }}-hub".to_owned()),
                    "<hub>",
                    "</hub>",
                ))),
            )]),
            templates: BTreeMap::from([(
                LLAMA3_HF_MODEL.to_owned(),
                ChatTemplateFile::Success("hub file template".to_owned()),
            )]),
        }),
    };
    assert_eq!(
        convert(LLAMA3_INSTRUCT_MODEL, &exchange, &BTreeMap::new(), &prompts).unwrap(),
        LLAMA3_RENDERED
    );
}

#[test]
fn fetch_deepseek_error_surfaces() {
    let exchange = messages(r#"[{"role":"user","content":"hi"}]"#);
    let prompts = empty_factory();
    let error = runtime()
        .block_on(convert_messages_to_prompt(PromptConversion {
            model: "deepseek_r1/deepseek.r1-v1:0",
            messages: &exchange,
            provider: Some(BedrockInvokeProvider::DeepseekR1),
            custom_prompt_dict: &BTreeMap::new(),
            prompts: &prompts,
            now: clock(),
        }))
        .unwrap_err();
    assert_eq!(error.to_string(), "No chat template found");
    assert!(matches!(
        error,
        ConversionError::Factory(FactoryError::NoChatTemplate)
    ));
}
