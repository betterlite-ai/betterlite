use betterlite_models::identity::provider_id::ProviderId;

use super::super::tags::{ChatConfigTag, ConfigConstruction};
use super::super::vertex::{self, VertexModelLists};
use super::harness::{CHAT_CATALOG, CallSpec, TEXT_CATALOG, make_world};

const VERTEX_CASES: &[(&str, ChatConfigTag, &str)] = &[
    (
        "gemini-2.5-pro",
        ChatConfigTag::VertexGemini,
        "VertexGeminiConfig",
    ),
    (
        "gemini-listed",
        ChatConfigTag::VertexGemini,
        "VertexGeminiConfig",
    ),
    (
        "GEMINI-2.5",
        ChatConfigTag::VertexAiLlama3,
        "VertexAILlama3Config",
    ),
    (
        "claude-3-opus",
        ChatConfigTag::VertexAiAnthropic,
        "VertexAIAnthropicConfig",
    ),
    (
        "gpt-oss-120b",
        ChatConfigTag::VertexAiGptOss,
        "VertexAIGPTOSSTransformation",
    ),
    (
        "mistral-large",
        ChatConfigTag::VertexAiMistral,
        "VertexAIMistralConfig",
    ),
    (
        "codestral-2501",
        ChatConfigTag::CodestralTextCompletion,
        "CodestralTextCompletionConfig",
    ),
    (
        "mistralai/codestral-2",
        ChatConfigTag::CodestralTextCompletion,
        "CodestralTextCompletionConfig",
    ),
    (
        "gemini-claude",
        ChatConfigTag::VertexGemini,
        "VertexGeminiConfig",
    ),
    (
        "claude-gpt-oss",
        ChatConfigTag::VertexAiAnthropic,
        "VertexAIAnthropicConfig",
    ),
    (
        "codestral-latest",
        ChatConfigTag::VertexAiLlama3,
        "VertexAILlama3Config",
    ),
    (
        "dual-listed",
        ChatConfigTag::VertexAiMistral,
        "VertexAIMistralConfig",
    ),
    (
        "jamba-1.5",
        ChatConfigTag::VertexAiAi21,
        "VertexAIAi21Config",
    ),
    (
        "llama-3-70b",
        ChatConfigTag::VertexAiLlama3,
        "VertexAILlama3Config",
    ),
];

fn vertex_lists() -> (Vec<String>, Vec<String>) {
    (
        vec![
            "mistral-large".to_owned(),
            "codestral-2501".to_owned(),
            "mistralai/codestral-2".to_owned(),
            "gemini-listed".to_owned(),
            "dual-listed".to_owned(),
        ],
        vec!["jamba-1.5".to_owned(), "dual-listed".to_owned()],
    )
}

pub(super) fn vertex_selection() {
    let mut world = make_world(CHAT_CATALOG, TEXT_CATALOG);
    let (mistral, ai21) = vertex_lists();
    world.set_vertex_lists(mistral, ai21);
    for &(model, tag, class) in VERTEX_CASES {
        let selection = world
            .select(&CallSpec::plain(ProviderId::VertexAi, model))
            .unwrap()
            .unwrap();
        assert_eq!(selection.tag, tag, "{model}");
        assert_eq!(selection.tag.class_name(), class, "{model}");
        assert_eq!(
            selection.construction,
            ConfigConstruction::New(vec![model.to_owned()]),
            "{model}"
        );
    }
    assert_eq!(
        vertex::select_config("mistral-large", VertexModelLists::default()),
        ChatConfigTag::VertexAiLlama3
    );
    let empty = make_world(&[], &[]);
    assert_eq!(
        empty
            .select(&CallSpec::plain(ProviderId::VertexAi, "mistral-large"))
            .unwrap()
            .map(|selection| selection.tag),
        Some(ChatConfigTag::VertexAiLlama3)
    );
}
