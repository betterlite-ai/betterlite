use std::collections::{BTreeMap, BTreeSet};

use betterlite_openai::api::rerank_contracts::records::RerankDocument;
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, Object, OpenAiConfig, OpenAiSettings,
};
use betterlite_openai::reasoning_models::DropParams;
use betterlite_openai::together_ai_chat::{
    LookupFailed, OptionMapping, TogetherAiChatConfig, TogetherCapabilities, TogetherFlag,
};
use betterlite_openai::together_ai_completion::{
    TogetherCompletionError, transform_text_completion_request,
};
use betterlite_openai::together_ai_cost::{
    CostMap, PricingCallType, TogetherPricingTiers, model_has_registry_pricing, pricing_category,
};
use betterlite_openai::together_ai_rerank::{
    RerankCall, ReturnDocuments, TogetherRerankError, request, transform_response,
};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

pub(crate) struct Op;

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Input {
    model_cost: BTreeMap<String, BTreeMap<String, Value>>,
    chat_supported: Vec<ChatSupportedStep>,
    chat_map: Vec<ChatMapStep>,
    chat_messages: Vec<ChatMessagesStep>,
    completion: Vec<CompletionStep>,
    rerank_requests: Vec<RerankRequestStep>,
    rerank_responses: Vec<RerankResponseStep>,
    cost_categories: Vec<CategoryStep>,
    cost_registry: Vec<RegistryStep>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChatSupportedStep {
    model: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChatMapStep {
    model: String,
    params: Object,
    #[serde(default)]
    optional_params: Object,
    #[serde(default)]
    drop_params: bool,
    #[serde(default)]
    global_drop_params: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChatMessagesStep {
    messages: Vec<ChatMessage>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompletionStep {
    model: String,
    messages: Vec<Value>,
    #[serde(default)]
    optional_params: Object,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RerankRequestStep {
    model: String,
    query: String,
    documents: Vec<RerankDocument>,
    #[serde(default)]
    top_n: Option<i64>,
    #[serde(default)]
    rank_fields: Option<Vec<String>>,
    #[serde(default, with = "serde_with::rust::double_option")]
    return_documents: Option<Option<bool>>,
    #[serde(default)]
    max_chunks_per_doc: Option<i64>,
    api_key: String,
    api_base: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RerankResponseStep {
    response: Value,
    next_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CategoryStep {
    model: String,
    #[serde(default)]
    call_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistryStep {
    model: String,
}

struct ModelCost<'map>(&'map BTreeMap<String, BTreeMap<String, Value>>);

impl TogetherCapabilities for ModelCost<'_> {
    fn check(&self, flag: TogetherFlag, model: &str) -> Result<bool, LookupFailed> {
        Ok(self
            .0
            .get(&format!("together_ai/{model}"))
            .or_else(|| self.0.get(model))
            .and_then(|entry| entry.get(flag.name()))
            .and_then(Value::as_bool)
            .unwrap_or(false))
    }

    fn registry_flag(&self, flag: TogetherFlag, model: &str) -> Option<bool> {
        self.0
            .get(&format!("together_ai/{model}"))
            .or_else(|| self.0.get(model))
            .and_then(|entry| entry.get(flag.name()))
            .and_then(Value::as_bool)
    }

    fn declared_reasoning_efforts(&self, model: &str) -> Option<Vec<String>> {
        let levels = self
            .0
            .get(&format!("together_ai/{model}"))
            .or_else(|| self.0.get(model))?
            .get("reasoning_effort_levels")?;
        let Value::Array(levels) = levels else {
            return None;
        };
        Some(
            levels
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
        )
    }
}

struct Priced(BTreeSet<String>);

impl CostMap for Priced {
    fn has_price_entry(&self, key: &str) -> bool {
        self.0.contains(key)
    }
}

fn empty_catalog() -> CatalogCapabilities {
    CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    }
}

fn supported_step(
    config: &TogetherAiChatConfig,
    step: &ChatSupportedStep,
    capabilities: &ModelCost<'_>,
) -> Value {
    let params = config.supported_params(&step.model, &empty_catalog(), capabilities);
    json!(params)
}

fn map_step(
    config: &TogetherAiChatConfig,
    step: &ChatMapStep,
    capabilities: &ModelCost<'_>,
) -> Value {
    let mut optional = step.optional_params.clone();
    let outcome = config.map_openai_params(
        OptionMapping {
            model: &step.model,
            non_default_params: &step.params,
            optional_params: &mut optional,
            catalog: &empty_catalog(),
            drop_params: DropParams {
                request: step.drop_params,
                global: step.global_drop_params,
            },
        },
        capabilities,
    );
    match outcome {
        Ok(()) => json!({"ok": true, "value": optional}),
        Err(error) => json!({"ok": false, "error": {"status": error.status_code()}}),
    }
}

fn messages_step(step: &ChatMessagesStep) -> Value {
    let mut messages = step.messages.clone();
    TogetherAiChatConfig::strip_internal_fields(&mut messages);
    serde_json::to_value(messages).unwrap_or(Value::Null)
}

fn completion_kind(error: &TogetherCompletionError) -> &'static str {
    match error {
        TogetherCompletionError::Integers => "integers",
        TogetherCompletionError::MultiplePrompts => "multiple_prompts",
        TogetherCompletionError::Prompt(_) => "prompt",
    }
}

fn completion_step(step: &CompletionStep) -> Value {
    let result = transform_text_completion_request(
        &step.model,
        &step.messages,
        step.optional_params.clone(),
    );
    match result {
        Ok(request) => json!({"ok": true, "value": request}),
        Err(error) => json!({"ok": false, "error": {"kind": completion_kind(&error)}}),
    }
}

fn rerank_kind(error: &TogetherRerankError) -> &'static str {
    match error {
        TogetherRerankError::MaxChunksPerDoc => "max_chunks",
        TogetherRerankError::Response { .. } => "response",
        TogetherRerankError::NoResults { .. } => "no_results",
        TogetherRerankError::MissingResultFields { .. } => "missing_fields",
        TogetherRerankError::InvalidUsage { .. } | TogetherRerankError::UsageField { .. } => {
            "invalid_usage"
        }
        TogetherRerankError::InvalidDocument { .. } => "invalid_document",
        TogetherRerankError::Score { .. } => "score",
        TogetherRerankError::Encode { .. } => "encode",
    }
}

fn rerank_request_step(step: &RerankRequestStep) -> Value {
    let key = SecretString::from(step.api_key.clone());
    let returned = match step.return_documents {
        None => ReturnDocuments::Default,
        Some(None) => ReturnDocuments::Null,
        Some(Some(value)) => ReturnDocuments::Value(value),
    };
    let call = RerankCall {
        model: &step.model,
        query: &step.query,
        documents: &step.documents,
        top_n: step.top_n,
        rank_fields: step.rank_fields.as_deref(),
        return_documents: returned,
        max_chunks_per_doc: step.max_chunks_per_doc,
        api_key: &key,
        api_base: &step.api_base,
    };
    match request(&call) {
        Ok(http) => json!({
            "ok": true,
            "value": {"url": http.url, "headers": http.headers, "body": http.body},
        }),
        Err(error) => json!({"ok": false, "error": {"kind": rerank_kind(&error)}}),
    }
}

fn rerank_response_step(step: &RerankResponseStep) -> Value {
    let next = step.next_id.clone();
    let next_id = move || Uuid::parse_str(&next).unwrap_or_default();
    match transform_response(&step.response, &next_id) {
        Ok(reply) => {
            json!({"ok": true, "value": serde_json::to_value(reply).unwrap_or(Value::Null)})
        }
        Err(error) => json!({"ok": false, "error": {"kind": rerank_kind(&error)}}),
    }
}

fn category_step(step: &CategoryStep) -> Value {
    let call_type = match step.call_type.as_deref() {
        Some("embedding") => PricingCallType::Embedding,
        Some("aembedding") => PricingCallType::Aembedding,
        Some(_other) => PricingCallType::Other,
        None => PricingCallType::Other,
    };
    json!(pricing_category(
        &step.model,
        call_type,
        &TogetherPricingTiers::default()
    ))
}

fn registry_step(step: &RegistryStep, priced: &Priced) -> Value {
    json!(model_has_registry_pricing(&step.model, priced))
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let config = TogetherAiChatConfig {
            base: OpenAiConfig {
                family: ConfigFamily::Other,
                api_key: None,
                headers: Headers::new(),
                settings: OpenAiSettings::default(),
            },
        };
        let capabilities = ModelCost(&input.model_cost);
        let priced = Priced(
            input
                .model_cost
                .iter()
                .filter(|(_, entry)| entry.contains_key("input_cost_per_token"))
                .map(|(key, _)| key.clone())
                .collect(),
        );
        Ok(json!({
            "chat_supported": input.chat_supported.iter().map(|step| supported_step(&config, step, &capabilities)).collect::<Vec<_>>(),
            "chat_map": input.chat_map.iter().map(|step| map_step(&config, step, &capabilities)).collect::<Vec<_>>(),
            "chat_messages": input.chat_messages.iter().map(messages_step).collect::<Vec<_>>(),
            "completion": input.completion.iter().map(completion_step).collect::<Vec<_>>(),
            "rerank_requests": input.rerank_requests.iter().map(rerank_request_step).collect::<Vec<_>>(),
            "rerank_responses": input.rerank_responses.iter().map(rerank_response_step).collect::<Vec<_>>(),
            "cost_categories": input.cost_categories.iter().map(category_step).collect::<Vec<_>>(),
            "cost_registry": input.cost_registry.iter().map(|step| registry_step(step, &priced)).collect::<Vec<_>>(),
        }))
    }
}
