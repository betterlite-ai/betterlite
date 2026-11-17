use std::collections::BTreeMap;

use betterlite_foundry::rerank::auth::{ValidateEnvironmentRequest, validate_environment};
use betterlite_foundry::rerank::error::FoundryRerankError;
use betterlite_foundry::rerank::reply::{
    FoundryRerankReply, HIDDEN_MODEL_PARAM, MODEL_GROUP_PARAM, transform_reply,
};
use betterlite_foundry::rerank::request::{FoundryRerankRequest, transform_request};
use betterlite_foundry::rerank::url::complete_url;
use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::RerankResponseResult;
use betterlite_openai::api::responses_shapes::wire::WireDecodeLimits;
use betterlite_openai::cohere_rerank::CohereRerankError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use secrecy::SecretString;
use serde_json::{Value, json};

mod input;
mod support;
mod tests;

use input::{AuthStep, Input, ReplyStep, RequestStep, UrlStep};
use support::Offline;

const LIMITS: WireDecodeLimits = WireDecodeLimits {
    max_json_bytes: 0x0010_0000,
    max_event_bytes: 0x0004_0000,
    max_items: 10_000,
};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let sources = Offline;
        Ok(json!({
            "urls": input.urls.iter().map(url_step).collect::<Vec<Value>>(),
            "requests": input.requests.iter().map(request_step).collect::<Vec<Value>>(),
            "replies": input.replies.iter().map(reply_step).collect::<Vec<Value>>(),
            "auths": input
                .auths
                .iter()
                .map(|step| auth_step(step, &sources))
                .collect::<Vec<Value>>(),
        }))
    }
}

fn url_step(step: &UrlStep) -> Value {
    complete_url(step.api_base.as_deref()).map_or_else(
        |_| json!({"error": {"status": null}}),
        |url| json!({"url": url}),
    )
}

fn request_step(step: &RequestStep) -> Value {
    let request = FoundryRerankRequest {
        model: step.model.clone(),
        query: step.query.clone(),
        documents: step.documents.clone(),
        top_n: step.top_n,
        rank_fields: step.rank_fields.clone(),
        return_documents: step.return_documents,
        max_chunks_per_doc: step.max_chunks_per_doc,
    };
    transform_request(&request).map_or_else(
        |_| json!({"error": {"status": null}}),
        |body| json!({"body": body}),
    )
}

fn reply_step(step: &ReplyStep) -> Value {
    let hidden = step
        .model_group
        .as_ref()
        .map_or_else(BTreeMap::new, |group| {
            BTreeMap::from([(MODEL_GROUP_PARAM.to_owned(), Json::Text(group.clone()))])
        });
    match transform_reply(step.status, step.body.as_bytes(), LIMITS, hidden) {
        Ok(reply) => reply_value(&reply),
        Err(FoundryRerankError::Cohere(CohereRerankError::ResponseJson { status, .. })) => {
            json!({"error": {"status": status}})
        }
        Err(_) => json!({"error": {"status": null}}),
    }
}

fn reply_value(reply: &FoundryRerankReply) -> Value {
    json!({
        "id": reply.id,
        "results": reply.results.as_ref().map(|entries| {
            Value::Array(entries.iter().map(result_value).collect())
        }),
        "meta": reply.meta,
        "hidden_model": hidden_model(reply),
        "hidden_model_present": reply.hidden_params.contains_key(HIDDEN_MODEL_PARAM),
    })
}

fn result_value(entry: &RerankResponseResult) -> Value {
    json!({
        "index": entry.index,
        "relevance_score": entry.relevance_score.0.normalize().to_string(),
        "document": entry.document,
    })
}

fn hidden_model(reply: &FoundryRerankReply) -> Value {
    match reply.hidden_params.get(HIDDEN_MODEL_PARAM) {
        Some(Json::Text(model)) => json!(model),
        Some(_) | None => Value::Null,
    }
}

fn auth_step(step: &AuthStep, sources: &Offline) -> Value {
    let api_key = step
        .api_key
        .as_ref()
        .map(|text| SecretString::from(text.clone()));
    let global_api_key = step
        .global_api_key
        .as_ref()
        .map(|text| SecretString::from(text.clone()));
    let request = ValidateEnvironmentRequest {
        headers: step.headers.clone(),
        api_key: api_key.as_ref(),
        params: step.params.as_ref(),
        environment: &step.environment,
        global_api_key: global_api_key.as_ref(),
        refresh_enabled: false,
        sources,
    };
    validate_environment(&request).map_or_else(
        |_| json!({"error": {"status": null}}),
        |headers| json!({"headers": headers}),
    )
}
