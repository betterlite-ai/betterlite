#![cfg(test)]

mod stream;

use std::convert::Infallible;
use std::future::{Future, ready};

use betterlite_models::decisions::Json as DecisionJson;
use betterlite_models::full_model_metadata::parameters::SupportedParameterSource;
use betterlite_models::model_capability_predicates::{
    CapabilityPredicates, CapabilityRequest, NamedCapability,
};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::system_placement::text_parts::CONVERTED_SYSTEM_NOTE;
use indexmap::IndexMap;
use serde_json::json;

use crate::api::messages_chat_output::records::ResponseRecord;
use crate::api::messages_chat_tools_schema::tool_choice::ToolChoiceError;
use crate::api::messages_context_clear::result::{
    AppliedEdit, CompactionBlock, CompactionKind, PolyfillResult, UsageIteration, UsageKind,
};

use super::adapter::AnthropicAdapter;
use super::compose::translate_anthropic_to_openai;
use super::error::TransformError;
use super::settings::AdapterSettings;

const MODEL_REQUIRED: &str = "Bad Request: model is required for Anthropic Messages Request";
const MESSAGES_REQUIRED: &str = "Bad Request: messages is required for Anthropic Messages Request";

// LiteLLM reads the capability from its pinned model map, so a test says which models it flags.
struct Capabilities {
    models: Vec<String>,
    providers: Vec<String>,
    probes: Vec<String>,
    provider_probes: Vec<Option<String>>,
}

impl Capabilities {
    fn new(models: &[&str]) -> Self {
        Self {
            models: models.iter().map(|model| String::from(*model)).collect(),
            providers: Vec::new(),
            probes: Vec::new(),
            provider_probes: Vec::new(),
        }
    }
}

impl CapabilityPredicates for Capabilities {
    fn supports_factory(
        &mut self,
        request: &CapabilityRequest<'_>,
        key: &str,
    ) -> impl Future<Output = bool> + Send {
        self.probes.push(request.model.to_owned());
        self.provider_probes
            .push(request.custom_llm_provider.map(str::to_owned));
        let flagged = key == NamedCapability::MidConversationSystem.key()
            && (self.models.iter().any(|model| model == request.model)
                || self
                    .providers
                    .iter()
                    .any(|provider| request.custom_llm_provider == Some(provider.as_str())));
        ready(flagged)
    }

    fn declared_value_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = Option<String>> + Send {
        ready(None)
    }

    fn is_explicitly_disabled_factory(
        &mut self,
        _request: &CapabilityRequest<'_>,
        _key: &str,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }

    fn supports_native_streaming(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = DecisionJson> + Send {
        ready(DecisionJson::Null)
    }

    fn supports_response_schema(
        &mut self,
        _request: &CapabilityRequest<'_>,
    ) -> impl Future<Output = bool> + Send {
        ready(false)
    }
}

// The provider's declared parameters are unknown here, so every option step reads them as absent.
struct Declared;

impl SupportedParameterSource for Declared {
    type Error = Infallible;

    fn supported_openai_params(
        &self,
        _model: &str,
        _custom_llm_provider: Option<&str>,
    ) -> Result<Option<Vec<String>>, Infallible> {
        Ok(None)
    }
}

fn json(source: &str) -> Json {
    serde_json::from_str(source).expect("a valid JSON fixture")
}

fn object(source: &str) -> Object {
    serde_json::from_str(source).expect("a valid object fixture")
}

const fn adapter() -> AnthropicAdapter {
    AnthropicAdapter::new()
}

async fn wrap(
    kwargs: &mut Object,
    capabilities: &mut Capabilities,
) -> Result<(Object, IndexMap<String, String>), TransformError> {
    let declared = Declared;
    adapter()
        .translate_completion_input_params_with_tool_mapping(
            kwargs,
            &AdapterSettings::new(&declared),
            capabilities,
        )
        .await
}

async fn compose(
    request: &mut Object,
    preserve_midturn_system: bool,
    capabilities: &mut Capabilities,
) -> Result<(Object, IndexMap<String, String>), TransformError> {
    let declared = Declared;
    let mut settings = AdapterSettings::new(&declared);
    settings.preserve_midturn_system = preserve_midturn_system;
    translate_anthropic_to_openai(request, &settings, capabilities).await
}

#[tokio::test]
async fn both_required_keys_leave_kwargs_before_either_is_checked() {
    let mut missing_model = object(r#"{"messages": [{"role": "user", "content": "hi"}]}"#);
    let model_error = wrap(&mut missing_model, &mut Capabilities::new(&[]))
        .await
        .expect_err("a request without a model cannot compose");
    assert!(matches!(
        model_error,
        TransformError::MissingKey { key: "model" }
    ));
    assert_eq!(model_error.to_string(), "'model'");
    assert_eq!(
        missing_model,
        object(r#"{"messages": [{"role": "user", "content": "hi"}]}"#),
        "the messages stay in kwargs when model is missing"
    );

    let mut missing_messages = object(r#"{"model": "gpt-4o"}"#);
    let messages_error = wrap(&mut missing_messages, &mut Capabilities::new(&[]))
        .await
        .expect_err("a request without messages cannot compose");
    assert!(matches!(
        messages_error,
        TransformError::MissingKey { key: "messages" }
    ));
    assert_eq!(messages_error.to_string(), "'messages'");
    assert!(
        missing_messages.is_empty(),
        "model was popped before the missing messages key raised"
    );
}

#[tokio::test]
async fn empty_required_keys_raise_litellms_value_errors() {
    let mut empty_model =
        object(r#"{"model": "", "messages": [{"role": "user", "content": "hi"}]}"#);
    let model_error = wrap(&mut empty_model, &mut Capabilities::new(&[]))
        .await
        .expect_err("an empty model is refused");
    assert!(matches!(model_error, TransformError::ModelRequired));
    assert_eq!(model_error.to_string(), MODEL_REQUIRED);

    let mut empty_messages = object(r#"{"model": "gpt-4o", "messages": []}"#);
    let messages_error = wrap(&mut empty_messages, &mut Capabilities::new(&[]))
        .await
        .expect_err("empty messages are refused");
    assert!(matches!(messages_error, TransformError::MessagesRequired));
    assert_eq!(messages_error.to_string(), MESSAGES_REQUIRED);

    let declared = Declared;
    let mut convenience =
        object(r#"{"model": "", "messages": [{"role": "user", "content": "hi"}]}"#);
    let convenience_error = adapter()
        .translate_completion_input_params(
            &mut convenience,
            AdapterSettings::new(&declared),
            &mut Capabilities::new(&[]),
        )
        .await
        .expect_err("the convenience wrapper validates the same way");
    assert!(matches!(convenience_error, TransformError::ModelRequired));
}

#[tokio::test]
async fn non_text_required_keys_are_typed_errors() {
    let mut numeric_model =
        object(r#"{"model": 7, "messages": [{"role": "user", "content": "hi"}]}"#);
    let numeric_error = wrap(&mut numeric_model, &mut Capabilities::new(&[]))
        .await
        .expect_err("a numeric model is refused");
    assert!(matches!(numeric_error, TransformError::ModelNotText));

    let mut text_messages = object(r#"{"model": "gpt-4o", "messages": "hi"}"#);
    let text_error = wrap(&mut text_messages, &mut Capabilities::new(&[]))
        .await
        .expect_err("a string messages value is refused");
    assert!(matches!(text_error, TransformError::MessagesNotList));
}

#[tokio::test]
async fn composed_chat_request_carries_every_translated_option() {
    let mut kwargs = object(
        r#"{
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hello"}],
            "system": "guide",
            "max_tokens": 12,
            "metadata": {"user_id": "alice"},
            "litellm_metadata": {"tag": "t"},
            "stop_sequences": ["END"],
            "tool_choice": {"type": "any"}
        }"#,
    );
    let (request, mapping) = wrap(&mut kwargs, &mut Capabilities::new(&[]))
        .await
        .expect("the request composes");
    assert!(mapping.is_empty(), "tools sent nothing to map");
    assert_eq!(
        request,
        object(
            r#"{
                "model": "gpt-4o",
                "messages": [
                    {"role": "system", "content": "guide"},
                    {"role": "user", "content": "hello"}
                ],
                "max_tokens": 12,
                "user": "alice",
                "metadata": {"tag": "t"},
                "tool_choice": "required",
                "stop": ["END"]
            }"#
        )
    );
    assert_eq!(
        kwargs,
        object(
            r#"{
                "system": "guide",
                "max_tokens": 12,
                "metadata": {"user_id": "alice"},
                "litellm_metadata": {"tag": "t"},
                "stop_sequences": ["END"],
                "tool_choice": {"type": "any"}
            }"#
        ),
        "the wrapper pops only the two required keys from the caller's kwargs"
    );
}

#[tokio::test]
async fn composition_removes_litellm_metadata_from_the_request_it_was_given() {
    let mut request = object(
        r#"{
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hi"}],
            "litellm_metadata": {"tag": "t"}
        }"#,
    );
    let (kwargs, _mapping) = compose(&mut request, false, &mut Capabilities::new(&[]))
        .await
        .expect("the request composes");
    assert!(
        request.get("litellm_metadata").is_none(),
        "the metadata moved out of the request record"
    );
    assert_eq!(kwargs.get("metadata"), Some(&json(r#"{"tag": "t"}"#)));
}

#[tokio::test]
async fn an_explicit_stop_overwrites_the_translated_stop_sequences() {
    let mut kwargs = object(
        r#"{
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hi"}],
            "stop_sequences": ["END"],
            "stop": ["LAST"]
        }"#,
    );
    let (request, _mapping) = wrap(&mut kwargs, &mut Capabilities::new(&[]))
        .await
        .expect("the request composes");
    assert_eq!(request.get("stop"), Some(&json(r#"["LAST"]"#)));
    assert!(
        request.get("stop_sequences").is_none(),
        "the translated key is not copied through a second time"
    );
}

#[tokio::test]
async fn thinking_budget_and_adaptive_effort_reach_the_composed_request() {
    let mut budgeted = object(
        r#"{
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hi"}],
            "thinking": {"type": "enabled", "budget_tokens": 9000}
        }"#,
    );
    let (request, _budgeted_mapping) = wrap(&mut budgeted, &mut Capabilities::new(&[]))
        .await
        .expect("the budgeted request composes");
    assert_eq!(request.get("reasoning_effort"), Some(&json(r#""high""#)));
    assert!(
        request.get("thinking").is_none(),
        "a non-Claude target receives the budgeted effort, not the thinking block"
    );

    let mut adaptive = object(
        r#"{
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hi"}],
            "thinking": {"type": "adaptive"},
            "output_config": {"effort": "low"}
        }"#,
    );
    let (adaptive_request, _adaptive_mapping) = wrap(&mut adaptive, &mut Capabilities::new(&[]))
        .await
        .expect("the adaptive request composes");
    assert_eq!(
        adaptive_request.get("reasoning_effort"),
        Some(&json(r#""low""#)),
        "the declared adaptive tier wins over the adaptive default"
    );
    assert!(
        adaptive_request.get("output_config").is_none(),
        "the effort tier is translated, so the config does not copy through"
    );
}

#[tokio::test]
async fn output_format_and_output_config_format_reach_the_composed_request() {
    let expected = json(
        r#"{
            "type": "json_schema",
            "json_schema": {
                "name": "structured_output",
                "schema": {"type": "object"},
                "strict": true
            }
        }"#,
    );
    let mut legacy = object(
        r#"{
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hi"}],
            "output_format": {"type": "json_schema", "schema": {"type": "object"}}
        }"#,
    );
    let (legacy_request, _legacy_mapping) = wrap(&mut legacy, &mut Capabilities::new(&[]))
        .await
        .expect("the output_format request composes");
    assert_eq!(legacy_request.get("response_format"), Some(&expected));

    let mut configured = object(
        r#"{
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hi"}],
            "output_config": {"format": {"type": "json_schema", "schema": {"type": "object"}}}
        }"#,
    );
    let (configured_request, _configured_mapping) =
        wrap(&mut configured, &mut Capabilities::new(&[]))
            .await
            .expect("the output_config.format request composes");
    assert_eq!(
        configured_request.get("response_format"),
        Some(&expected),
        "output_config.format translates the same way as the legacy field"
    );
    assert!(
        configured_request.get("output_config").is_none(),
        "the format is translated, so the config does not copy through"
    );
}

#[tokio::test]
async fn long_tool_names_roundtrip_through_the_returned_mapping() {
    let long = "a".repeat(65);
    let short = format!("{}_635361c4", "a".repeat(55));
    let tools = json(&format!(
        r#"[{{"name": "{long}", "input_schema": {{"type": "object"}}}}]"#
    ));
    let choice = json(&format!(r#"{{"type": "tool", "name": "{long}"}}"#));
    let messages = json(r#"[{"role": "user", "content": "hi"}]"#);
    let mut kwargs = Object::new();
    kwargs.insert("model".to_owned(), Json::Text("gpt-4o".to_owned()));
    kwargs.insert("messages".to_owned(), messages.clone());
    kwargs.insert("tools".to_owned(), tools.clone());
    kwargs.insert("tool_choice".to_owned(), choice.clone());
    let (request, mapping) = wrap(&mut kwargs, &mut Capabilities::new(&[]))
        .await
        .expect("the request composes");
    assert_eq!(mapping.len(), 1);
    assert_eq!(mapping.get(&short), Some(&long));
    assert_eq!(
        request.get("tools"),
        Some(&json(&format!(
            r#"[{{"type": "function", "function": {{"name": "{short}"}}}}]"#
        )))
    );
    assert_eq!(
        request.get("tool_choice"),
        Some(&json(&format!(
            r#"{{"type": "function", "function": {{"name": "{short}"}}}}"#
        )))
    );

    let record: ResponseRecord = serde_json::from_value(json!({
        "id": "chat_a",
        "model": "gpt-4o",
        "choices": [{
            "finish_reason": "tool_calls",
            "message": {
                "content": null,
                "tool_calls": [{
                    "id": "call_1",
                    "function": {"name": short, "arguments": "{\"x\":1}"}
                }]
            }
        }]
    }))
    .expect("the response record decodes");
    let message = adapter()
        .translate_completion_output_params(&record, Some(&mapping), None)
        .expect("the response translates");
    let value = serde_json::to_value(&message).expect("the message encodes");
    assert_eq!(value["content"][0]["type"], "tool_use");
    assert_eq!(value["content"][0]["name"], long);
    assert_eq!(value["stop_reason"], "tool_use");

    let mut again = Object::new();
    again.insert("model".to_owned(), Json::Text("gpt-4o".to_owned()));
    again.insert("messages".to_owned(), messages);
    again.insert("tools".to_owned(), tools);
    again.insert("tool_choice".to_owned(), choice);
    let declared = Declared;
    let convenience = adapter()
        .translate_completion_input_params(
            &mut again,
            AdapterSettings::new(&declared),
            &mut Capabilities::new(&[]),
        )
        .await
        .expect("the convenience wrapper composes");
    assert_eq!(
        convenience, request,
        "the convenience wrapper returns the same request without the map"
    );
}

#[tokio::test]
async fn preserve_midturn_system_reaches_the_conversation_translation() {
    let request_json = r#"{
        "model": "gpt-4o",
        "messages": [
            {"role": "user", "content": "hi"},
            {"role": "system", "content": "late"}
        ]
    }"#;
    let mut preserved = object(request_json);
    let mut preserved_capabilities = Capabilities::new(&[]);
    let (kept, _preserved_mapping) = compose(&mut preserved, true, &mut preserved_capabilities)
        .await
        .expect("the request composes");
    assert_eq!(
        kept.get("messages"),
        Some(&json(
            r#"[{"role": "user", "content": "hi"}, {"role": "system", "content": "late"}]"#
        ))
    );
    assert!(
        preserved_capabilities.probes.is_empty(),
        "the preserve flag decides before any capability lookup"
    );

    let mut converted = object(request_json);
    let mut folded_capabilities = Capabilities::new(&[]);
    let (folded, _folded_mapping) = compose(&mut converted, false, &mut folded_capabilities)
        .await
        .expect("the request composes");
    assert_eq!(
        folded.get("messages"),
        Some(&json(&format!(
            r#"[{{"role": "user", "content": "hi"}},
                {{"role": "user", "content": [
                    {{"type": "text", "text": "{CONVERTED_SYSTEM_NOTE}"}},
                    {{"type": "text", "text": "late"}}
                ]}}]"#
        ))),
        "an unsupported trailing system turn becomes a user turn"
    );
    assert_eq!(
        folded_capabilities.probes,
        ["gpt-4o".to_owned()],
        "the model reaches the capability lookup"
    );
}

#[tokio::test]
async fn the_convenience_wrapper_composes_without_a_provider() {
    let request_json = r#"{
        "model": "gpt-4o",
        "messages": [
            {"role": "user", "content": "hi"},
            {"role": "system", "content": "late"}
        ]
    }"#;
    let declared = Declared;
    let mut settings = AdapterSettings::new(&declared);
    settings.custom_llm_provider = Some("openai");
    let mut routed_capabilities = Capabilities::new(&[]);
    routed_capabilities.providers.push("openai".to_owned());
    let mut routed = object(request_json);
    let (kept, _mapping) = adapter()
        .translate_completion_input_params_with_tool_mapping(
            &mut routed,
            &settings,
            &mut routed_capabilities,
        )
        .await
        .expect("the routed request composes");
    assert_eq!(
        kept.get("messages"),
        Some(&json(
            r#"[{"role": "user", "content": "hi"}, {"role": "system", "content": "late"}]"#
        )),
        "the named provider decides the trailing system turn"
    );
    assert_eq!(
        routed_capabilities.provider_probes,
        [Some("openai".to_owned())],
        "the routed wrapper sends the provider it was given"
    );

    let mut capabilities = Capabilities::new(&[]);
    capabilities.providers.push("openai".to_owned());
    let mut unrouted = object(request_json);
    let folded = adapter()
        .translate_completion_input_params(&mut unrouted, settings, &mut capabilities)
        .await
        .expect("the convenience request composes");
    assert_eq!(
        folded.get("messages"),
        Some(&json(&format!(
            r#"[{{"role": "user", "content": "hi"}},
                {{"role": "user", "content": [
                    {{"type": "text", "text": "{CONVERTED_SYSTEM_NOTE}"}},
                    {{"type": "text", "text": "late"}}
                ]}}]"#
        ))),
        "the convenience wrapper names no provider, so the trailing system turn is converted"
    );
    assert_eq!(
        capabilities.provider_probes,
        [None],
        "the convenience wrapper composes as if no provider was named"
    );
}

#[tokio::test]
async fn option_step_errors_surface_through_the_composition() {
    let mut kwargs = object(
        r#"{
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "hi"}],
            "tool_choice": {"type": "bogus"}
        }"#,
    );
    let error = wrap(&mut kwargs, &mut Capabilities::new(&[]))
        .await
        .expect_err("an unknown tool_choice type is refused");
    assert!(matches!(
        error,
        TransformError::ToolChoice(ToolChoiceError::Unsupported)
    ));
}

#[tokio::test]
async fn composed_message_response_wraps_the_chat_completion() {
    let record: ResponseRecord = serde_json::from_value(json!({
        "id": "chat_a",
        "model": "gpt-4o",
        "choices": [{
            "finish_reason": "stop",
            "message": {"content": "hello"}
        }],
        "usage": {"prompt_tokens": 10, "completion_tokens": 2}
    }))
    .expect("the response record decodes");
    let message = adapter()
        .translate_completion_output_params(&record, None, None)
        .expect("the response translates");
    assert_eq!(
        serde_json::to_value(&message).expect("the message encodes"),
        json!({
            "id": "chat_a",
            "type": "message",
            "role": "assistant",
            "model": "gpt-4o",
            "stop_sequence": null,
            "usage": {"input_tokens": 10, "output_tokens": 2},
            "content": [{"type": "text", "text": "hello"}],
            "stop_reason": "end_turn",
            "stop_details": null
        })
    );
}

#[tokio::test]
async fn polyfill_output_reaches_the_message_response() {
    let record: ResponseRecord = serde_json::from_value(json!({
        "id": "chat_a",
        "model": "gpt-4o",
        "choices": [{
            "finish_reason": "stop",
            "message": {"content": "hello"}
        }],
        "usage": {"prompt_tokens": 10, "completion_tokens": 2}
    }))
    .expect("the response record decodes");
    let polyfill = PolyfillResult {
        messages: Vec::new(),
        system: None,
        applied_edits: vec![AppliedEdit {
            edit_type: Some("clear_tool_uses_20250919".to_owned()),
            cleared_tool_uses: Some(1),
            ..AppliedEdit::default()
        }],
        compaction_block: Some(CompactionBlock {
            kind: CompactionKind::Compaction,
            content: Some("sum".to_owned()),
            signature: Some("s".to_owned()),
        }),
        iterations_usage: Some(vec![UsageIteration {
            kind: UsageKind::Compaction,
            input_tokens: Some(4),
            output_tokens: Some(1),
        }]),
    };
    let message = adapter()
        .translate_completion_output_params(&record, None, Some(&polyfill))
        .expect("the response translates");
    let value = serde_json::to_value(&message).expect("the message encodes");
    assert_eq!(
        value["content"][0],
        json!({"type": "compaction", "content": "sum", "signature": "s"}),
        "the compaction block leads the content"
    );
    assert_eq!(
        value["context_management"]["applied_edits"],
        json!([{"type": "clear_tool_uses_20250919", "cleared_tool_uses": 1}])
    );
    assert_eq!(
        value["usage"]["iterations"],
        json!([
            {"type": "compaction", "input_tokens": 4, "output_tokens": 1},
            {"type": "message", "input_tokens": 10, "output_tokens": 2}
        ]),
        "the summary iteration comes before the message iteration"
    );
}
