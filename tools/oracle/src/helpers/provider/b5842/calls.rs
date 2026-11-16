use betterlite_openai::api::audio_file_input::ProcessedAudioFile;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::passthrough::transformation::BasePassthroughConfig as _;
use betterlite_openai::passthrough::transformation::types::{
    PassthroughTarget, PassthroughUrlRequest,
};
use betterlite_openai::watsonx_audio::WatsonxAudioConfig;
// ast-grep-ignore: no-long-names
use betterlite_openai::watsonx_audio::reply::decode_watsonx_transcription_reply as decode_audio;
use betterlite_openai::watsonx_auth::WatsonxAuth;
use betterlite_openai::watsonx_auth::types::WatsonxParams;
use betterlite_openai::watsonx_passthrough::WatsonxPassthroughConfig;
use betterlite_provider::api::rerank_contracts::{
    Headers, MapCohereRerankParams, Params, RerankProviderResponse,
};
use betterlite_provider::watsonx_embed::WatsonxEmbeddingConfig;
use betterlite_provider::watsonx_embed::reply::decode_watsonx_embedding_reply;
use betterlite_provider::watsonx_embed::request::WatsonxEmbeddingBody;
use betterlite_provider::watsonx_rerank::WatsonxRerankConfig;
use betterlite_provider::watsonx_rerank::params::map_cohere_rerank_params;
use betterlite_provider::watsonx_rerank::request::transform_rerank_request;
use secrecy::ExposeSecret as _;
use serde_json::{Value, json};
use uuid::Uuid;

use super::NoTransport;
use super::input::Step;
use super::raised::Raised;

type Outcome = Result<Value, Raised>;

pub(super) fn embed(auth: &WatsonxAuth<NoTransport>, step: &Step) -> Outcome {
    let (url, request) = embed_request(auth, step)?;
    Ok(json!({"url": url, "request": request, "reply": embed_reply(step)?}))
}

fn embed_request(
    auth: &WatsonxAuth<NoTransport>,
    step: &Step,
) -> Result<(String, WatsonxEmbeddingBody), Raised> {
    let environment = &auth.environment;
    let config = WatsonxEmbeddingConfig { environment };
    let input = step.input.as_ref().ok_or_else(Raised::default)?;
    let mut options: WatsonxParams = step.options.clone().into_iter().collect();
    let url = config.get_complete_url(step.api_base.as_deref(), &mut options)?;
    Ok((
        url,
        config.transform_embedding_request(&step.model, input, options)?,
    ))
}

fn embed_reply(step: &Step) -> Outcome {
    let Some(text) = &step.reply else {
        return Ok(Value::Null);
    };
    let decoded = decode_watsonx_embedding_reply(text.as_bytes())?;
    let response = WatsonxEmbeddingConfig::transform_embedding_response(&step.model, decoded);
    let usage = response.usage;
    Ok(json!({
        "object": response.object,
        "data": response.data,
        "usage": {
            "prompt_tokens": usage.prompt_tokens,
            "completion_tokens": usage.completion_tokens,
            "total_tokens": usage.total_tokens,
        },
    }))
}

pub(super) async fn rerank(auth: &WatsonxAuth<NoTransport>, step: &Step) -> Outcome {
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
    let headers = config
        .validate_environment(
            step.headers.clone(),
            step.api_key.as_ref(),
            &mut auth_options,
        )
        .await?;
    let request = transform_rerank_request(&step.model, &mapped, &auth.environment)?;
    let reply = rerank_reply(&config, step)?;
    Ok(json!({
        "map": mapped,
        "auth": {"headers": headers, "options": auth_options},
        "request": request,
        "reply": reply,
    }))
}

fn rerank_reply(config: &WatsonxRerankConfig<'_, NoTransport>, step: &Step) -> Outcome {
    let Some(text) = step.reply.as_deref() else {
        return Ok(Value::Null);
    };
    let Some(status) = step.status else {
        return Ok(Value::Null);
    };
    let raw = RerankProviderResponse {
        status_code: status,
        headers: Headers::new(),
        body: text.as_bytes().to_vec(),
    };
    Ok(json!(config.transform_rerank_response(&raw)?))
}

pub(super) fn audio(auth: &WatsonxAuth<NoTransport>, step: &Step) -> Outcome {
    let config = WatsonxAudioConfig { auth };
    let url = config.get_complete_url(step.api_base.as_deref(), &step.options)?;
    let request = audio_request(&config, step)?;
    Ok(json!({"url": url, "request": request, "reply": audio_reply(step)?}))
}

fn audio_request(config: &WatsonxAudioConfig<'_, NoTransport>, step: &Step) -> Outcome {
    let Some(file) = &step.file else {
        return Ok(Value::Null);
    };
    let processed = ProcessedAudioFile {
        file_content: file.content.as_bytes().to_vec(),
        filename: file.filename.clone(),
        content_type: file.content_type.clone(),
    };
    let built = config.transform_audio_request(&step.model, processed, &step.options)?;
    let data: Object = built.fields.into_iter().collect();
    Ok(json!({
        "data": data,
        "file": {
            "filename": built.file.filename,
            "content_type": built.file.content_type,
            "size": built.file.file_content.len(),
        },
    }))
}

fn audio_reply(step: &Step) -> Outcome {
    let reply = step
        .reply
        .as_deref()
        .map(|text| decode_audio(text.as_bytes()))
        .transpose()?;
    Ok(json!(reply))
}

pub(super) fn passthrough(auth: &WatsonxAuth<NoTransport>, step: &Step) -> Outcome {
    let config = WatsonxPassthroughConfig {
        environment: &auth.environment,
    };
    let url = passthrough_url(&config, step)?;
    Ok(json!({
        "url": url,
        "stream": config.is_streaming_request(&step.endpoint, &step.data),
        "key": config.get_api_key(step.api_key.as_ref())?.expose_secret(),
    }))
}

fn passthrough_url(config: &WatsonxPassthroughConfig<'_>, step: &Step) -> Outcome {
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
    Ok(json!({"url": url.as_str(), "base": base}))
}
