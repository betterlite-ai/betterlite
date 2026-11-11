use std::collections::BTreeMap;

use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::PromptFactory;

use crate::aws_endpoints::provider::BedrockInvokeProvider;
use crate::invoke_primitives::prompt::{
    ConversionError, ConvertedPrompt, CustomPromptEntry, PromptConversion,
    convert_messages_to_prompt,
};

use super::support::{clock, factory, messages, runtime};

const TITAN: &str = "\n\nUser: <admin>s</admin>\n\nUser: u\n\nBot: a";
const CLAUDE: &str = "\n\nHuman: <admin>s</admin>\n\nHuman: u\n\nAssistant: a";
const MISTRAL: &str = "<s>[INST] \ns [/INST]\n[INST] u [/INST]\n a</s> ";
const LLAMA2: &str = "<s>[INST] <<SYS>>\ns\n<</SYS>>\n [/INST]\n[INST] u [/INST]\na\n</s>";
const DEEPSEEK: &str = "<\u{ff5c}begin\u{2581}of\u{2581}sentence\u{ff5c}>s<\u{ff5c}User\u{ff5c}>u<\u{ff5c}Assistant\u{ff5c}><think>\n";
const CONVERSATION: &str = r#"[{"role":"system","content":"s"},{"role":"user","content":"u"},{"role":"assistant","content":"a"}]"#;
const EXCHANGE: &str = r#"[{"role":"system","content":"s"},{"role":"user","content":"u"}]"#;

fn no_custom() -> BTreeMap<String, CustomPromptEntry> {
    BTreeMap::new()
}

fn convert<'text>(
    model: &'text str,
    messages: &'text [Message],
    provider: Option<BedrockInvokeProvider>,
    custom: &'text BTreeMap<String, CustomPromptEntry>,
    prompts: &'text PromptFactory,
) -> Result<ConvertedPrompt, ConversionError> {
    runtime().block_on(convert_messages_to_prompt(PromptConversion {
        model,
        messages,
        provider,
        custom_prompt_dict: custom,
        prompts,
        now: clock(),
    }))
}

fn prompt(
    model: &str,
    messages: &[Message],
    provider: BedrockInvokeProvider,
    prompts: &PromptFactory,
) -> String {
    convert(model, messages, Some(provider), &no_custom(), prompts)
        .unwrap()
        .prompt
}

pub(super) fn prompt_template_selection() {
    anthropic_and_amazon();
    mistral_and_llama();
    deepseek_template();
    cohere_history();
    fallback_concatenation();
    custom_entry();
    non_text_family();
}

fn anthropic_and_amazon() {
    let conversation = messages(CONVERSATION);
    let factory = factory();
    assert_eq!(
        prompt(
            "anthropic.claude-v2",
            &conversation,
            BedrockInvokeProvider::Anthropic,
            &factory
        ),
        CLAUDE
    );
    assert_eq!(
        prompt(
            "amazon.titan-text-express-v1",
            &conversation,
            BedrockInvokeProvider::Amazon,
            &factory
        ),
        TITAN
    );
}

fn mistral_and_llama() {
    let conversation = messages(CONVERSATION);
    let factory = factory();
    assert_eq!(
        prompt(
            "mistral.mixtral-8x7b-instruct",
            &conversation,
            BedrockInvokeProvider::Mistral,
            &factory
        ),
        MISTRAL
    );
    assert_eq!(
        prompt(
            "meta.llama2-13b-chat",
            &conversation,
            BedrockInvokeProvider::Meta,
            &factory
        ),
        LLAMA2
    );
    assert_eq!(
        prompt(
            "llama/meta.llama2-13b-chat",
            &conversation,
            BedrockInvokeProvider::Llama,
            &factory
        ),
        LLAMA2
    );
}

fn deepseek_template() {
    assert_eq!(
        prompt(
            "deepseek_r1/deepseek.r1-v1:0",
            &messages(EXCHANGE),
            BedrockInvokeProvider::DeepseekR1,
            &factory()
        ),
        DEEPSEEK
    );
}

fn cohere_history() {
    let converted = convert(
        "cohere.command-r-v1:0",
        &messages(r#"[{"role":"user","content":"hi"}]"#),
        Some(BedrockInvokeProvider::Cohere),
        &no_custom(),
        &factory(),
    )
    .unwrap();
    assert_eq!(converted.prompt, "hi");
    assert_eq!(converted.chat_history, Some(Vec::new()));
}

fn fallback_concatenation() {
    assert_eq!(
        prompt(
            "ai21.j2-mid-v1",
            &messages(CONVERSATION),
            BedrockInvokeProvider::Ai21,
            &factory()
        ),
        "sua"
    );
    assert_eq!(
        prompt(
            "ai21.j2-mid-v1",
            &messages(r#"[{"role":"user","content":[{"type":"text","text":"a"}]}]"#),
            BedrockInvokeProvider::Ai21,
            &factory()
        ),
        "[{'type': 'text', 'text': 'a'}]"
    );
    let unconverted = convert(
        "ai21.j2-mid-v1",
        &messages(CONVERSATION),
        None,
        &no_custom(),
        &factory(),
    )
    .unwrap();
    assert_eq!(unconverted.prompt, "sua");
}

fn custom_entry() {
    let custom: BTreeMap<String, CustomPromptEntry> = serde_json::from_str(
        r#"{"invoke/anthropic.claude-v2":{"roles":{"user":{"pre_message":"[INST] ","post_message":" [/INST]"}},"initial_prompt_value":"<s>","final_prompt_value":"</s>","bos_token":"ignored","eos_token":"ignored"}}"#,
    )
    .unwrap();
    let exchange = messages(r#"[{"role":"user","content":"hi"}]"#);
    let customised = convert(
        "invoke/anthropic.claude-v2",
        &exchange,
        Some(BedrockInvokeProvider::Anthropic),
        &custom,
        &factory(),
    )
    .unwrap();
    assert_eq!(customised.prompt, "<s>[INST] hi [/INST]</s>");
    assert_eq!(customised.chat_history, None);
    // a key that differs by any character is not custom, so the family runs
    assert_eq!(
        convert(
            "anthropic.claude-v2",
            &exchange,
            Some(BedrockInvokeProvider::Anthropic),
            &custom,
            &factory(),
        )
        .unwrap()
        .prompt,
        "\n\nHuman: hi\n\nAssistant: "
    );
}

fn non_text_family() {
    let error = convert(
        "tiiuae/falcon-7b",
        &messages(CONVERSATION),
        Some(BedrockInvokeProvider::Meta),
        &no_custom(),
        &factory(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        ConversionError::NotText {
            provider: "bedrock"
        }
    ));
}
