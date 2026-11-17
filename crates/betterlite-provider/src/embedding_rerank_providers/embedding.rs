use std::sync::Arc;

use betterlite_core::embedding_rerank_providers::{EmbeddingConfig, embedding_config};
use betterlite_openai::cohere_embedding::CohereEmbeddingError;
use betterlite_openai::cohere_embedding::request as cohere_request;
use betterlite_openai::embedding::{EmbeddingInput, EmbeddingRequest};
use betterlite_openai::infinity_embedding;
use betterlite_openai::jina_embedding;
use betterlite_openai::openai_base_config::{Headers, Object as BaseObject};
use betterlite_openai::voyage_embedding::{self, VoyageEmbeddingVariant};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::assets;
use betterlite_tokenizer::engine::tiktoken::{Encoding, TiktokenEngine};
use secrecy::ExposeSecret;

use super::Common;
use super::error::FamilyError;
use super::request::{encode, http_headers, plain, text_headers, url};
use super::{Call, CohereEmbeddingDecode, Decode, EmbeddingRerankPrepared, JinaEmbeddingDecode};

pub fn prepare(
    common: &Common<'_>,
    request: &EmbeddingRequest,
) -> Result<EmbeddingRerankPrepared, FamilyError> {
    let config =
        embedding_config(common.provider, common.model).ok_or(FamilyError::Unsupported {
            provider: common.provider.to_string(),
        })?;
    let non_default = non_default_params(request)?;
    let call = match config {
        EmbeddingConfig::Cohere => cohere(common, request, &non_default)?,
        EmbeddingConfig::Voyage
        | EmbeddingConfig::VoyageContextual
        | EmbeddingConfig::VoyageMultimodal => voyage(common, request, &non_default)?,
        EmbeddingConfig::JinaAi => jina(common, request, &non_default)?,
        EmbeddingConfig::Infinity => infinity(common, request, &non_default)?,
    };
    Ok(EmbeddingRerankPrepared(call))
}

// `headers = headers or extra_headers` followed by `headers.update(extra_headers)`, and no globals
fn merged_headers(supplied: Option<&Headers>, extra: &Headers) -> Headers {
    let mut merged = match supplied.filter(|headers| !headers.is_empty()) {
        Some(headers) => headers.clone(),
        None => extra.clone(),
    };
    merged.extend(extra.clone());
    merged
}

fn cohere(
    common: &Common<'_>,
    request: &EmbeddingRequest,
    non_default: &Object,
) -> Result<Call, FamilyError> {
    let key = common.settings.resolve_api_key(common.api_key);
    let headers = cohere_request::validate_environment(
        merged_headers(common.headers, common.extra_headers),
        key.as_ref(),
    );
    let endpoint = cohere_request::get_complete_url(common.api_base);
    let texts = match &request.input {
        EmbeddingInput::Text(text) => vec![text.clone()],
        EmbeddingInput::Texts(texts) => texts.clone(),
        EmbeddingInput::Tokens(_) | EmbeddingInput::TokenLists(_) => {
            return Err(CohereEmbeddingError::InputNotText.into());
        }
    };
    let mapped = cohere_request::map_openai_params(non_default, Object::new());
    let body = cohere_request::transform_embedding_request(common.model, &request.input, &mapped)?;
    let decode = CohereEmbeddingDecode {
        model: common.model.to_owned(),
        api_key: key,
        request_data: body.clone(),
        input: texts,
        encoding: encoding()?,
    };
    Ok(Call {
        url: url("cohere", &endpoint)?,
        headers: http_headers(&headers)?,
        body: encode(&body)?,
        timeout: common.timeout,
        decode: Decode::CohereEmbedding(Box::new(decode)),
    })
}

// LiteLLM counts the input with litellm.encoding when Cohere bills neither input nor image tokens
fn encoding() -> Result<Engine, FamilyError> {
    let vocabulary = assets::bundled(Encoding::Cl100kBase).ok_or(FamilyError::Vocabulary)?;
    let engine = TiktokenEngine::from_bpe_file(Encoding::Cl100kBase, &vocabulary)
        .map_err(CohereEmbeddingError::Encoding)?;
    Ok(Engine::Tiktoken(Arc::new(engine)))
}

fn voyage(
    common: &Common<'_>,
    request: &EmbeddingRequest,
    non_default: &Object,
) -> Result<Call, FamilyError> {
    let variant = VoyageEmbeddingVariant::for_model(common.model);
    let authorization = voyage_embedding::voyage_credentials(
        common.api_key.map(ExposeSecret::expose_secret),
        |name| common.environment.lookup(name),
    );
    // LiteLLM passes no headers to the Voyage handler, and its validate_environment clears them
    let headers = text_headers(voyage_embedding::voyage_headers(
        BaseObject::new(),
        authorization,
    ));
    // the text and contextual Voyage configs support encoding_format and dimensions; multimodal
    // supports dimensions alone, and the request builder renames that one
    let supported: &[&str] = match variant {
        VoyageEmbeddingVariant::Multimodal => &["dimensions"],
        VoyageEmbeddingVariant::Text | VoyageEmbeddingVariant::Contextual => {
            &["encoding_format", "dimensions"]
        }
    };
    let supplied: BaseObject = plain(non_default)?;
    let options = BaseObject::from_iter(supported.iter().filter_map(|name| {
        supplied
            .get(*name)
            .map(|value| ((*name).to_owned(), value.clone()))
    }));
    let given =
        serde_json::to_value(&request.input).map_err(|source| FamilyError::Encode { source })?;
    let body = voyage_embedding::request(common.model, given, &options, variant);
    Ok(Call {
        url: url(
            "voyage",
            &voyage_embedding::endpoint(common.api_base, variant),
        )?,
        headers: http_headers(&headers)?,
        body: encode(&body)?,
        timeout: common.timeout,
        decode: Decode::VoyageEmbedding(variant),
    })
}

fn jina(
    common: &Common<'_>,
    request: &EmbeddingRequest,
    non_default: &Object,
) -> Result<Call, FamilyError> {
    let credentials =
        jina_embedding::resolve_credentials(common.api_base, common.api_key, common.host);
    let endpoint = jina_embedding::get_complete_url(Some(credentials.api_base.as_str()));
    let supplied: BaseObject = plain(non_default)?;
    let mut mapped = BaseObject::new();
    jina_embedding::map_openai_params(&supplied, &mut mapped);
    let given =
        serde_json::to_value(&request.input).map_err(|source| FamilyError::Encode { source })?;
    let body = jina_embedding::transform_embedding_request(common.model, &given, &mapped);
    let decode = JinaEmbeddingDecode {
        api_key: credentials.api_key.clone(),
        request_data: body.clone(),
    };
    Ok(Call {
        url: url("jina", &endpoint)?,
        headers: http_headers(&jina_embedding::validate_environment(
            &Headers::new(),
            credentials.api_key.as_ref(),
        ))?,
        body: encode(&body)?,
        timeout: common.timeout,
        decode: Decode::JinaEmbedding(Box::new(decode)),
    })
}

fn infinity(
    common: &Common<'_>,
    request: &EmbeddingRequest,
    non_default: &Object,
) -> Result<Call, FamilyError> {
    let endpoint = infinity_embedding::get_complete_url(common.api_base)?;
    let supplied: BaseObject = plain(non_default)?;
    let mut mapped = BaseObject::new();
    infinity_embedding::map_openai_params(&supplied, &mut mapped);
    let given =
        serde_json::to_value(&request.input).map_err(|source| FamilyError::Encode { source })?;
    let body = infinity_embedding::transform_embedding_request(common.model, &given, &mapped);
    Ok(Call {
        url: url("infinity", &endpoint)?,
        headers: http_headers(&infinity_embedding::validate_environment(
            &Headers::new(),
            common.api_key,
            common.host,
        ))?,
        body: encode(&body)?,
        timeout: common.timeout,
        decode: Decode::InfinityEmbedding,
    })
}

// LiteLLM's get_optional_params_embeddings receives only the parameters the caller set
fn non_default_params(request: &EmbeddingRequest) -> Result<Object, FamilyError> {
    let mut fields: Object = plain(request)?;
    fields.shift_remove("model");
    fields.shift_remove("input");
    fields.retain(|_, value| !matches!(value, Json::Null));
    Ok(fields)
}
