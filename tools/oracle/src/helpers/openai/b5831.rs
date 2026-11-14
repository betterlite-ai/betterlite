use std::collections::{BTreeMap, BTreeSet};

use betterlite_openai::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, OpenAiConfig, OpenAiSettings,
};
use betterlite_openai::reasoning_models::DropParams;
use betterlite_openai::together_ai_chat::{
    LookupFailed, OptionMapping, TogetherAiChatConfig, TogetherCapabilities, TogetherFlag,
};
use betterlite_openai::together_ai_completion::{
    TogetherCompletionError, transform_completion_request,
};
use betterlite_openai::together_ai_cost::{
    CostMap, PricingCallType, TogetherPricingTiers, model_has_registry_pricing, pricing_category,
};
use betterlite_openai::together_ai_rerank::{
    RerankCall, ReturnDocuments, TogetherRerankError, request, transform_response,
};
use secrecy::SecretString;
use serde_json::{Value, json};
use uuid::Uuid;

mod inputs;

use inputs::{
    CategoryStep, ChatMapStep, ChatMessagesStep, CompletionStep, Input, RerankRequestStep,
    RerankResponseStep, ReturnDocumentsStep,
};

pub(crate) struct Op;

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
    let result =
        transform_completion_request(&step.model, &step.messages, step.optional_params.clone());
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
        TogetherRerankError::Encode { .. } | TogetherRerankError::UsageDecode { .. } => "encode",
    }
}

fn outcome(result: Result<Value, &'static str>) -> Value {
    match result {
        Ok(value) => json!({"ok": true, "value": value}),
        Err(kind) => json!({"ok": false, "error": {"kind": kind}}),
    }
}

fn rerank_request_step(step: &RerankRequestStep) -> Value {
    let key = SecretString::from(step.api_key.clone());
    let returned = match step.return_documents {
        ReturnDocumentsStep::Absent => ReturnDocuments::Default,
        ReturnDocumentsStep::Null => ReturnDocuments::Null,
        ReturnDocumentsStep::Flag(value) => ReturnDocuments::Value(value),
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
    outcome(
        request(&call)
            .map(|http| json!({"url": http.url, "headers": http.headers, "body": http.body}))
            .map_err(|error| rerank_kind(&error)),
    )
}

fn rerank_response_step(step: &RerankResponseStep) -> Value {
    let next = step.next_id.clone();
    let next_id = move || Uuid::parse_str(&next).unwrap_or_default();
    outcome(
        transform_response(&step.response, &next_id)
            .map(|reply| serde_json::to_value(reply).unwrap_or(Value::Null))
            .map_err(|error| rerank_kind(&error)),
    )
}

fn category_step(step: &CategoryStep) -> Value {
    let call_type = match step.call_type.as_deref() {
        Some("embedding") => PricingCallType::Embedding,
        Some("aembedding") => PricingCallType::Aembedding,
        Some(_) | None => PricingCallType::Other,
    };
    json!(pricing_category(
        &step.model,
        call_type,
        &TogetherPricingTiers::default()
    ))
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
            "chat_supported": input.chat_supported.iter().map(|step| json!(config.supported_params(&step.model, &empty_catalog(), &capabilities))).collect::<Vec<_>>(),
            "chat_map": input.chat_map.iter().map(|step| map_step(&config, step, &capabilities)).collect::<Vec<_>>(),
            "chat_messages": input.chat_messages.iter().map(messages_step).collect::<Vec<_>>(),
            "completion": input.completion.iter().map(completion_step).collect::<Vec<_>>(),
            "rerank_requests": input.rerank_requests.iter().map(rerank_request_step).collect::<Vec<_>>(),
            "rerank_responses": input.rerank_responses.iter().map(rerank_response_step).collect::<Vec<_>>(),
            "cost_categories": input.cost_categories.iter().map(category_step).collect::<Vec<_>>(),
            "cost_registry": input.cost_registry.iter().map(|step| json!(model_has_registry_pricing(&step.model, &priced))).collect::<Vec<_>>(),
        }))
    }
}
