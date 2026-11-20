use std::collections::BTreeMap;

use betterlite_foundry::rerank::reply::{
    FoundryRerankReply, HIDDEN_MODEL_PARAM, MODEL_GROUP_PARAM,
};
use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::RerankResponseResult;
use betterlite_openai::api::responses_shapes::wire::WireDecodeLimits;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_provider::foundry_resource_dispatch::rerank::{Call, Dispatch, Outcome};
use betterlite_provider::foundry_resource_dispatch::{
    FoundryResourceOperation, FoundryResourceResult, dispatch,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::support::{OTHER, failure, runtime};

const LIMITS: WireDecodeLimits = WireDecodeLimits {
    max_json_bytes: 0x0010_0000,
    max_event_bytes: 0x0004_0000,
    max_items: 10_000,
};

#[derive(Debug, Deserialize)]
pub(super) struct Input {
    #[serde(default = "default_status")]
    status: u16,
    #[serde(default)]
    body: String,
    model_group: Option<String>,
}

const fn default_status() -> u16 {
    200
}

// B556's reply keeps the provider's ranking indices and the hidden model group
pub(super) fn run(input: &Input) -> HelperResult {
    let runtime = runtime()?;
    let operation = FoundryResourceOperation::Rerank(Box::new(Dispatch {
        call: Call::Reply {
            status: input.status,
            body: input.body.as_bytes(),
            limits: Box::new(LIMITS),
            hidden_params: Box::new(hidden_params(input.model_group.as_deref())),
        },
    }));
    match runtime.block_on(dispatch(operation)) {
        Ok(FoundryResourceResult::Rerank(Outcome::Reply(reply))) => Ok(reply_value(&reply)),
        Ok(_) => Err(failure("Variant", OTHER)),
        Err(error) => Err(failure("LeafFailure", error)),
    }
}

fn hidden_params(model_group: Option<&str>) -> BTreeMap<String, Json> {
    model_group.map_or_else(BTreeMap::new, |group| {
        BTreeMap::from([(MODEL_GROUP_PARAM.to_owned(), Json::Text(group.to_owned()))])
    })
}

fn reply_value(reply: &FoundryRerankReply) -> Value {
    let results = reply
        .results
        .as_ref()
        .map(|entries| Value::Array(entries.iter().map(result_value).collect()));
    json!({
        "id": reply.id,
        "results": results,
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
