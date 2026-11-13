use betterlite_openai::api::rerank_contracts::records::RerankResponse;
use secrecy::SecretString;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{Case, Input};
use betterlite_provider::api::rerank_contracts::{
    BaseRerankConfig as _, Headers, MapCohereRerankParams, Params, RerankLogging,
    RerankProviderResponse, ResponseTransform,
};
use betterlite_provider::nvidia_nim_rerank::ranking::NvidiaNimRankingConfig;
use betterlite_provider::nvidia_nim_rerank::retrieval::NvidiaNimRerankConfig;
use betterlite_provider::nvidia_nim_rerank::{
    NvidiaNimRerankError, NvidiaNimRerankSelection, select_rerank_config,
};

fn retrieval() -> NvidiaNimRerankConfig {
    NvidiaNimRerankConfig {
        env_api_key: None,
        global_api_key: None,
        new_id: || Uuid::from_u128(7),
    }
}

fn ranking() -> NvidiaNimRankingConfig {
    NvidiaNimRankingConfig {
        retrieval: retrieval(),
        top_n: std::sync::Mutex::new(None),
    }
}

fn selection_for(case: &Case) -> NvidiaNimRerankSelection {
    if case.config == "ranking" {
        NvidiaNimRerankSelection::Ranking(ranking())
    } else {
        NvidiaNimRerankSelection::Retrieval(retrieval())
    }
}

fn mapper<'call>(
    case: &'call Case,
    non_default: &'call mut Params,
) -> MapCohereRerankParams<'call> {
    MapCohereRerankParams {
        non_default_params: non_default,
        model: &case.model,
        drop_params: false,
        query: &case.query,
        documents: &case.documents,
        custom_llm_provider: Some("nvidia_nim"),
        top_n: case.top_n,
        rank_fields: None,
        return_documents: Some(true),
        max_chunks_per_doc: None,
        max_tokens_per_doc: None,
        instruction: None,
    }
}

fn mapped(case: &Case) -> Result<Params, NvidiaNimRerankError> {
    let mut non_default = case.non_default.clone().unwrap_or_default();
    let request = mapper(case, &mut non_default);
    match selection_for(case) {
        NvidiaNimRerankSelection::Retrieval(config) => config.map_cohere_rerank_params(request),
        NvidiaNimRerankSelection::Ranking(config) => config.map_cohere_rerank_params(request),
    }
}

fn url_of(selection: &NvidiaNimRerankSelection, api_base: Option<&str>, model: &str) -> String {
    match selection {
        NvidiaNimRerankSelection::Retrieval(config) => {
            config.get_complete_url(api_base, model, None)
        }
        NvidiaNimRerankSelection::Ranking(config) => config.get_complete_url(api_base, model, None),
    }
}

pub(crate) fn select(input: &Input) -> Value {
    let selected: Vec<String> = input
        .models
        .iter()
        .map(|model| url_of(&select_rerank_config(model), None, model))
        .collect();
    let urls: Vec<String> = input
        .urls
        .iter()
        .map(|case| {
            url_of(
                &select_rerank_config(&case.model),
                case.api_base.as_deref(),
                &case.model,
            )
        })
        .collect();
    json!({"selected": selected, "urls": urls})
}

pub(crate) fn params(cases: &[Case], header_cases: &[Case]) -> Value {
    let mut mapped = Vec::with_capacity(cases.len());
    for case in cases {
        mapped.push(match mapped(case) {
            Ok(found) => json!(found),
            Err(_) => Value::Null,
        });
    }
    let found: Vec<Value> = header_cases
        .iter()
        .map(|case| {
            let call_key = case.api_key.as_ref().map(SecretString::from);
            let env_key = case
                .env_key
                .as_ref()
                .filter(|key| !key.is_empty())
                .map(SecretString::from);
            let global_key = case
                .global_key
                .as_ref()
                .filter(|key| !key.is_empty())
                .map(SecretString::from);
            match NvidiaNimRerankConfig::validate_environment_with_keys(
                case.headers.clone(),
                call_key.as_ref(),
                env_key.as_ref(),
                global_key.as_ref(),
            ) {
                Ok(merged) => json!({"ok": true, "headers": merged}),
                Err(_) => json!({"ok": false}),
            }
        })
        .collect();
    json!({"mapped": mapped, "headers": found})
}

pub(crate) fn request(cases: &[Case]) -> Value {
    let bodies: Vec<Value> = cases
        .iter()
        .map(|case| {
            let selection = selection_for(case);
            let Ok(optional) = mapped(case) else {
                return json!({"ok": false});
            };
            let outcome = match &selection {
                NvidiaNimRerankSelection::Retrieval(config) => {
                    config.transform_rerank_request(&case.model, &optional, &Headers::new(), None)
                }
                NvidiaNimRerankSelection::Ranking(config) => {
                    config.transform_rerank_request(&case.model, &optional, &Headers::new(), None)
                }
            };
            match outcome {
                Ok(body) => json!({"ok": true, "body": body}),
                Err(_) => json!({"ok": false}),
            }
        })
        .collect();
    json!({"bodies": bodies})
}

pub(crate) fn response(cases: &[Case]) -> Value {
    let mut responses = Vec::with_capacity(cases.len());
    for case in cases {
        responses.push(one_response(case));
    }
    json!({"responses": responses})
}

fn one_response(case: &Case) -> Value {
    let selection = selection_for(case);
    let provider_response = RerankProviderResponse {
        status_code: case.status,
        headers: case.headers.clone(),
        body: case.body.as_bytes().to_vec(),
    };
    let mut logging = RerankLogging::default();
    let optional = case.optional.clone().unwrap_or_default();
    let litellm_params = Params::new();
    let request = ResponseTransform {
        model: &case.model,
        raw_response: &provider_response,
        logging: &mut logging,
        api_key: None,
        request_data: &case.request_data,
        optional_params: &optional,
        litellm_params: &litellm_params,
    };
    let outcome = match selection {
        NvidiaNimRerankSelection::Retrieval(config) => {
            config.transform_rerank_response(RerankResponse::default(), request)
        }
        NvidiaNimRerankSelection::Ranking(config) => {
            config.transform_rerank_response(RerankResponse::default(), request)
        }
    };
    match outcome {
        Ok(response) => serde_json::to_value(response).unwrap_or(Value::Null),
        Err(_) => json!({"ok": false}),
    }
}
