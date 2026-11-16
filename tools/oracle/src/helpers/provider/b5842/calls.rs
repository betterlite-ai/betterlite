use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::passthrough::transformation::BasePassthroughConfig;
use betterlite_openai::passthrough::transformation::types::{
    PassthroughTarget, PassthroughUrlRequest,
};
use betterlite_openai::watsonx_audio::WatsonxAudioConfig;
use betterlite_openai::watsonx_auth::WatsonxAuth;
use betterlite_openai::watsonx_auth::transport::WatsonxTransport;
use betterlite_openai::watsonx_auth::types::WatsonxParams;
use betterlite_openai::watsonx_passthrough::WatsonxPassthroughConfig;
use betterlite_provider::api::rerank_contracts::{
    Headers, MapCohereRerankParams, Params, RerankProviderResponse,
};
use betterlite_provider::watsonx_embed::WatsonxEmbeddingConfig;
use betterlite_provider::watsonx_embed::reply::decode_watsonx_embedding_reply;
use betterlite_provider::watsonx_rerank::WatsonxRerankConfig;
use betterlite_provider::watsonx_rerank::params::map_cohere_rerank_params;
use betterlite_provider::watsonx_rerank::request::transform_rerank_request;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Value, json};
use uuid::Uuid;

use super::input::Step;
use super::raised::Raised;

type Outcome = Result<Value, Raised>;

pub(super) fn embed<T: WatsonxTransport>(auth: &WatsonxAuth<T>, step: &Step) -> Outcome {
    let config = WatsonxEmbeddingConfig {
        environment: &auth.environment,
    };
    let input = step.input.as_ref().ok_or_else(Raised::default)?;
    let mut options: WatsonxParams = step.options.clone().into_iter().collect();
    let url = config.get_complete_url(step.api_base.as_deref(), &mut options)?;
    let body = config.transform_embedding_request(&step.model, input, options)?;
    let mut reply = Value::Null;
    if let Some(text) = &step.reply {
        let decoded = decode_watsonx_embedding_reply(text.as_bytes())?;
        let response = WatsonxEmbeddingConfig::transform_embedding_response(&step.model, decoded)?;
        let usage = response.usage;
        reply = json!({
            "object": response.object,
            "data": response.data,
            "usage": {
                "prompt_tokens": usage.prompt_tokens,
                "completion_tokens": usage.completion_tokens,
                "total_tokens": usage.total_tokens,
            },
        });
    }
    Ok(json!({"url": url, "request": body, "reply": reply}))
}

pub(super) async fn rerank<T: WatsonxTransport>(auth: &WatsonxAuth<T>, step: &Step) -> Outcome {
    let config = WatsonxRerankConfig {
        auth,
        new_id: Uuid::nil,
    };
    let mut options: Params = serde_json::from_value(Value::Object(step.options.clone()))
        .map_err(|_error| Raised::default())?;
    let mapped = map_cohere_rerank_params(&MapCohereRerankParams {
        non_default_params: &mut options,
        model: &step.model,
        drop_params: false,
        query: "",
        documents: &[],
        custom_llm_provider: None,
        top_n: None,
        rank_fields: None,
        return_documents: None,
        max_chunks_per_doc: None,
        max_tokens_per_doc: None,
        instruction: None,
    })?;
    let mut auth_options = options.clone();
    let api_key = step.api_key.clone().map(SecretString::from);
    let headers = config
        .validate_environment(step.headers.clone(), api_key.as_ref(), &mut auth_options)
        .await?;
    let request = transform_rerank_request(&step.model, mapped.clone(), &auth.environment)?;
    let mut reply = Value::Null;
    if let (Some(text), Some(status)) = (&step.reply, step.status) {
        let raw = RerankProviderResponse {
            status_code: status,
            headers: Headers::new(),
            body: text.as_bytes().to_vec(),
        };
        reply = json!(config.transform_rerank_response(&raw)?);
    }
    let lowered: Headers = headers
        .into_iter()
        .map(|(name, value)| (name.to_lowercase(), value))
        .collect();
    Ok(json!({
        "map": mapped,
        "auth": {"headers": lowered, "options": auth_options},
        "request": request,
        "reply": reply,
    }))
}

pub(super) fn audio<T: WatsonxTransport>(auth: &WatsonxAuth<T>, step: &Step) -> Outcome {
    let config = WatsonxAudioConfig { auth };
    let url = config.get_complete_url(step.api_base.as_deref(), &step.options)?;
    let mut request = Value::Null;
    if let Some(file) = &step.file {
        let processed = ProcessedAudioFile {
            file_content: file.content.as_bytes().to_vec(),
            filename: file.filename.clone(),
            content_type: file.content_type.clone(),
        };
        let built = config.transform_audio_request(&step.model, processed, &step.options)?;
        let data: Object = built.fields.into_iter().collect();
        request = json!({
            "data": data,
            "file": {
                "filename": built.file.filename,
                "content_type": built.file.content_type,
                "size": built.file.file_content.len(),
            },
        });
    }
    let mut reply = Value::Null;
    if let Some(text) = &step.reply {
        reply = json!(config.transform_audio_response(text.as_bytes())?);
    }
    Ok(json!({"url": url, "request": request, "reply": reply}))
}

pub(super) fn passthrough<T: WatsonxTransport>(auth: &WatsonxAuth<T>, step: &Step) -> Outcome {
    let config = WatsonxPassthroughConfig {
        environment: &auth.environment,
    };
    let empty = Object::new();
    let request = PassthroughUrlRequest {
        api_base: step.api_base.as_deref(),
        api_key: None,
        target: PassthroughTarget {
            model: "model",
            custom_llm_provider: "watsonx",
            endpoint: &step.endpoint,
        },
        request_query_params: step.query.as_ref(),
        litellm_params: &empty,
    };
    let (url, base) = config.get_complete_url(&request)?;
    let api_key = step.api_key.clone().map(SecretString::from);
    let key = config.get_api_key(api_key.as_ref())?;
    Ok(json!({
        "url": {"url": url.as_str(), "base": base},
        "stream": config.is_streaming_request(&step.endpoint, &step.data),
        "key": key.expose_secret(),
    }))
}
