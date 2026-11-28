use betterlite_http::url_path::quote_segment;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_secret_managers::secret_lookup::SecretLookup;

use super::error::EmbeddingCallError;
use super::{EmbeddingCall, ParamMap};
use crate::aws_endpoints::provider::{BedrockEmbeddingProvider, embedding_provider};
use crate::aws_endpoints::region::{LlmRegion, llm_region_name};
use crate::aws_endpoints::runtime::{BedrockEndpointType, RuntimeEndpointInput, runtime_endpoint};
use crate::aws_settings::sources::Credentials;
use crate::aws_settings::{AUTHENTICATION_PARAMS, bedrock_bearer_token};
use crate::bedrock_embedding_io::credentials::{
    CredentialRequest, LoadedCredentials, ManagedRegionWorker, load_credentials,
};
use crate::credentials::CredentialSource;
use crate::image_models::params::Params;
use crate::marengo_embeddings::request::{TransformRequest, transform_request as marengo_request};
use crate::nova_embeddings::request::transform_request as nova_request;
use crate::titan_embeddings::{cohere, g1, multimodal, v2};

#[derive(Debug)]
pub struct EmbeddingPlan {
    pub model: String,
    pub model_id: String,
    pub endpoint_url: String,
    pub provider: BedrockEmbeddingProvider,
    pub asynchronous: bool,
    pub data: Option<Object>,
    pub batch: Option<Vec<Object>>,
    pub credentials: Option<Credentials>,
    pub region: String,
}

struct Selection {
    model: String,
    model_id: String,
    region: String,
    provider: BedrockEmbeddingProvider,
    asynchronous: bool,
}

pub async fn plan<Source>(
    call: &mut EmbeddingCall<'_>,
    secrets: &SecretLookup,
    region_worker: &ManagedRegionWorker,
    source: &Source,
) -> Result<EmbeddingPlan, EmbeddingCallError>
where
    Source: CredentialSource,
{
    let loaded = credentials(call, secrets, region_worker, source).await?;
    let selected = select(call, &loaded)?;
    let inference_params = inference_params(&call.optional_params)?;
    let (data, batch) = bodies(&Bodies {
        provider: selected.provider,
        model: &selected.model,
        model_id: &selected.model_id,
        asynchronous: selected.asynchronous,
        input: call.input,
        inference_params: &inference_params,
        drop_params: call.drop_params,
    })?;
    let endpoint_url = endpoint_url(
        call,
        &selected.region,
        selected.asynchronous,
        &selected.model_id,
    )?;
    Ok(EmbeddingPlan {
        model: selected.model,
        model_id: selected.model_id,
        endpoint_url,
        provider: selected.provider,
        asynchronous: selected.asynchronous,
        data,
        batch,
        credentials: loaded.credentials,
        region: selected.region,
    })
}

async fn credentials<Source>(
    call: &mut EmbeddingCall<'_>,
    secrets: &SecretLookup,
    region_worker: &ManagedRegionWorker,
    source: &Source,
) -> Result<LoadedCredentials, EmbeddingCallError>
where
    Source: CredentialSource,
{
    let bearer = bedrock_bearer_token(call.api_key, call.environment.bearer_token);
    Ok(load_credentials(
        CredentialRequest {
            optional_params: &mut call.optional_params,
            environment: call.environment.variables,
            bearer_token: bearer.as_ref(),
        },
        secrets,
        region_worker,
        |settings, region| source.resolve(settings.clone(), region.to_owned()),
    )
    .await?)
}

fn select(
    call: &mut EmbeddingCall<'_>,
    loaded: &LoadedCredentials,
) -> Result<Selection, EmbeddingCallError> {
    let unencoded_id = optional_string(&mut call.optional_params, "model_id")?
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| call.model.to_owned());
    let model_id = quote_segment(&unencoded_id);
    // the loaded region is the explicit one LiteLLM hands to its region resolver
    let region = llm_region_name(LlmRegion {
        explicit: Some(&loaded.region),
        model: Some(call.model),
        model_id: Some(&unencoded_id),
        environment_region_name: None,
        environment_region: None,
        session_region: None,
    })?;
    let asynchronous = call.model.contains("async_invoke/");
    let model = if asynchronous {
        call.model.replacen("async_invoke/", "", 1)
    } else {
        call.model.to_owned()
    };
    let provider = embedding_provider(&model).ok_or_else(|| EmbeddingCallError::Provider {
        model: model.clone(),
    })?;
    Ok(Selection {
        model,
        model_id,
        region,
        provider,
        asynchronous,
    })
}

fn endpoint_url(
    call: &mut EmbeddingCall<'_>,
    region: &str,
    asynchronous: bool,
    model_id: &str,
) -> Result<String, EmbeddingCallError> {
    let configured = optional_string(&mut call.optional_params, "aws_bedrock_runtime_endpoint")?;
    let endpoint = runtime_endpoint(RuntimeEndpointInput {
        api_base: call.api_base,
        configured_endpoint: configured.as_deref(),
        environment_endpoint: call.environment.runtime_endpoint,
        region,
        endpoint_type: BedrockEndpointType::Runtime,
    });
    Ok(if asynchronous {
        format!("{}/async-invoke", endpoint.endpoint_url)
    } else {
        format!("{}/model/{model_id}/invoke", endpoint.endpoint_url)
    })
}

fn inference_params(params: &ParamMap) -> Result<Object, EmbeddingCallError> {
    let mut options = remaining_options(params)?;
    options.retain(|key, _| {
        !AUTHENTICATION_PARAMS
            .iter()
            .any(|name| key.eq_ignore_ascii_case(name))
            && key != "user"
    });
    Ok(options)
}

pub fn remaining_options(params: &ParamMap) -> Result<Object, EmbeddingCallError> {
    let mut options = Object::new();
    for (key, raw) in params {
        options.insert(key.clone(), serde_json::from_str(raw.get())?);
    }
    Ok(options)
}

fn optional_string(
    params: &mut ParamMap,
    field: &'static str,
) -> Result<Option<String>, EmbeddingCallError> {
    let Some(raw) = params.remove(field) else {
        return Ok(None);
    };
    serde_json::from_str::<Option<String>>(raw.get())
        .map_err(|source| EmbeddingCallError::Field { field, source })
}

struct Bodies<'text> {
    provider: BedrockEmbeddingProvider,
    model: &'text str,
    model_id: &'text str,
    asynchronous: bool,
    input: &'text [String],
    inference_params: &'text Object,
    drop_params: bool,
}

fn bodies(spec: &Bodies<'_>) -> Result<(Option<Object>, Option<Vec<Object>>), EmbeddingCallError> {
    let batch = match spec.provider {
        BedrockEmbeddingProvider::Cohere => {
            let data = cohere::transform_request(spec.model, spec.input, spec.inference_params);
            return Ok((Some(data), None));
        }
        // an amazon model outside the four titan ids builds no body; the caller's Unmapped check follows
        BedrockEmbeddingProvider::Amazon => {
            let Some(kind) = titan_kind(spec.model) else {
                return Ok((None, None));
            };
            let mut requests = Vec::with_capacity(spec.input.len());
            for text in spec.input {
                requests.push(match kind {
                    Titan::Multimodal => multimodal::transform_request(text, spec.inference_params),
                    Titan::G1 => g1::transform_request(text),
                    Titan::V2 => v2::transform_request(text, spec.inference_params)?,
                });
            }
            requests
        }
        BedrockEmbeddingProvider::Twelvelabs => async_bodies(spec, Family::Marengo)?,
        BedrockEmbeddingProvider::Nova => async_bodies(spec, Family::Nova)?,
    };
    Ok((None, Some(batch)))
}

#[derive(Clone, Copy)]
enum Titan {
    Multimodal,
    G1,
    V2,
}

#[derive(Clone, Copy)]
enum Family {
    Marengo,
    Nova,
}

fn titan_kind(model: &str) -> Option<Titan> {
    match model {
        "amazon.titan-embed-image-v1" => Some(Titan::Multimodal),
        "amazon.titan-embed-text-v1" | "amazon.titan-embed-g1-text-02" => Some(Titan::G1),
        "amazon.titan-embed-text-v2:0" => Some(Titan::V2),
        _ => None,
    }
}

fn async_bodies(spec: &Bodies<'_>, family: Family) -> Result<Vec<Object>, EmbeddingCallError> {
    let inference_params: Params =
        serde_json::from_value(serde_json::to_value(spec.inference_params)?)?;
    let output_s3_uri = spec
        .inference_params
        .get("output_s3_uri")
        .and_then(Json::as_str);
    let mut batch = Vec::with_capacity(spec.input.len());
    for text in spec.input {
        let request = match family {
            Family::Marengo => serde_json::to_value(marengo_request(TransformRequest {
                model: Some(spec.model),
                input: text,
                inference_params: &inference_params,
                async_invoke_route: spec.asynchronous,
                model_id: Some(spec.model_id),
                output_s3_uri,
                drop_params: spec.drop_params,
            })?)?,
            Family::Nova => serde_json::to_value(nova_request(
                text,
                &inference_params,
                spec.asynchronous,
                Some(spec.model_id),
                output_s3_uri,
            )?)?,
        };
        batch.push(serde_json::from_value(request)?);
    }
    Ok(batch)
}
