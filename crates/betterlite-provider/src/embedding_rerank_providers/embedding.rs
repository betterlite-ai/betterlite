use std::sync::Arc;
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::embedding_host_settings;
use betterlite_config::sections::embedding_provider_settings::Settings;
use betterlite_core::embedding_rerank_providers::{EmbeddingConfig, embedding_config};
use betterlite_models::identity::provider_id::ProviderId;
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
use secrecy::{ExposeSecret, SecretString};

use super::error::FamilyError;
use super::request::{encode, http_headers, plain, text_headers, url};
use super::{Call, CohereEmbeddingDecode, Decode, EmbeddingRerankPrepared, JinaEmbeddingDecode};

pub struct EmbeddingCallInput<'request> {
    pub provider: ProviderId,
    pub model: &'request str,
    pub request: &'request EmbeddingRequest,
    pub api_key: Option<&'request SecretString>,
    pub api_base: Option<&'request str>,
    // the caller's headers; only Cohere embedding forwards them
    pub headers: Option<&'request Headers>,
    // LiteLLM merges extra_headers into headers before the family arm, so these win the choice
    pub extra_headers: &'request Headers,
    pub settings: &'request Settings,
    pub host: &'request embedding_host_settings::Settings,
    pub environment: &'request dyn EnvironmentLookup,
    pub timeout: Duration,
}

struct Prepared {
    url: String,
    headers: Headers,
    body: Vec<u8>,
    decode: Decode,
}

pub fn prepare(input: &EmbeddingCallInput<'_>) -> Result<EmbeddingRerankPrepared, FamilyError> {
    let config = embedding_config(input.provider, input.model).ok_or(FamilyError::Unsupported {
        provider: input.provider.to_string(),
    })?;
    let non_default = non_default_params(input.request)?;
    let prepared = match config {
        EmbeddingConfig::Cohere => cohere(input, &non_default)?,
        EmbeddingConfig::Voyage
        | EmbeddingConfig::VoyageContextual
        | EmbeddingConfig::VoyageMultimodal => voyage(input, &non_default)?,
        EmbeddingConfig::JinaAi => jina(input, &non_default)?,
        EmbeddingConfig::Infinity => infinity(input, &non_default)?,
    };
    Ok(EmbeddingRerankPrepared(Call {
        url: url(family(config), &prepared.url)?,
        headers: http_headers(&prepared.headers)?,
        body: prepared.body,
        timeout: input.timeout,
        decode: prepared.decode,
    }))
}

// the name a failed URL parse is reported against
const fn family(config: EmbeddingConfig) -> &'static str {
    match config {
        EmbeddingConfig::Cohere => "cohere",
        EmbeddingConfig::Voyage
        | EmbeddingConfig::VoyageContextual
        | EmbeddingConfig::VoyageMultimodal => "voyage",
        EmbeddingConfig::JinaAi => "jina",
        EmbeddingConfig::Infinity => "infinity",
    }
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

fn cohere(input: &EmbeddingCallInput<'_>, non_default: &Object) -> Result<Prepared, FamilyError> {
    let key = input.settings.resolve_api_key(input.api_key);
    let headers = cohere_request::validate_environment(
        merged_headers(input.headers, input.extra_headers),
        key.as_ref(),
    );
    let endpoint = cohere_request::get_complete_url(input.api_base);
    let texts = match &input.request.input {
        EmbeddingInput::Text(text) => vec![text.clone()],
        EmbeddingInput::Texts(texts) => texts.clone(),
        EmbeddingInput::Tokens(_) | EmbeddingInput::TokenLists(_) => {
            return Err(CohereEmbeddingError::InputNotText.into());
        }
    };
    let mapped = cohere_request::map_openai_params(non_default, Object::new());
    let body =
        cohere_request::transform_embedding_request(input.model, &input.request.input, &mapped)?;
    let decode = CohereEmbeddingDecode {
        model: input.model.to_owned(),
        api_key: key,
        request_data: body.clone(),
        input: texts,
        encoding: encoding()?,
    };
    Ok(Prepared {
        url: endpoint,
        headers,
        body: encode(&body)?,
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

fn voyage(input: &EmbeddingCallInput<'_>, non_default: &Object) -> Result<Prepared, FamilyError> {
    let variant = VoyageEmbeddingVariant::for_model(input.model);
    let authorization = voyage_embedding::voyage_credentials(
        input.api_key.map(ExposeSecret::expose_secret),
        |name| input.environment.lookup(name),
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
    let given = serde_json::to_value(&input.request.input)
        .map_err(|source| FamilyError::Encode { source })?;
    let body = voyage_embedding::request(input.model, given, &options, variant);
    Ok(Prepared {
        url: voyage_embedding::endpoint(input.api_base, variant),
        headers,
        body: serde_json::to_vec(&body).map_err(|source| FamilyError::Encode { source })?,
        decode: Decode::VoyageEmbedding(variant),
    })
}

fn jina(input: &EmbeddingCallInput<'_>, non_default: &Object) -> Result<Prepared, FamilyError> {
    let credentials =
        jina_embedding::resolve_credentials(input.api_base, input.api_key, input.host);
    let endpoint = jina_embedding::get_complete_url(Some(credentials.api_base.as_str()));
    let supplied: BaseObject = plain(non_default)?;
    let mut mapped = BaseObject::new();
    jina_embedding::map_openai_params(&supplied, &mut mapped);
    let given = serde_json::to_value(&input.request.input)
        .map_err(|source| FamilyError::Encode { source })?;
    let body = jina_embedding::transform_embedding_request(input.model, &given, &mapped);
    let decode = JinaEmbeddingDecode {
        api_key: credentials.api_key.clone(),
        request_data: body.clone(),
    };
    Ok(Prepared {
        url: endpoint,
        headers: jina_embedding::validate_environment(
            &Headers::new(),
            credentials.api_key.as_ref(),
        ),
        body: serde_json::to_vec(&body).map_err(|source| FamilyError::Encode { source })?,
        decode: Decode::JinaEmbedding(Box::new(decode)),
    })
}

fn infinity(input: &EmbeddingCallInput<'_>, non_default: &Object) -> Result<Prepared, FamilyError> {
    let endpoint = infinity_embedding::get_complete_url(input.api_base)?;
    let supplied: BaseObject = plain(non_default)?;
    let mut mapped = BaseObject::new();
    infinity_embedding::map_openai_params(&supplied, &mut mapped);
    let given = serde_json::to_value(&input.request.input)
        .map_err(|source| FamilyError::Encode { source })?;
    let body = infinity_embedding::transform_embedding_request(input.model, &given, &mapped);
    Ok(Prepared {
        url: endpoint,
        headers: infinity_embedding::validate_environment(
            &Headers::new(),
            input.api_key,
            input.host,
        ),
        body: serde_json::to_vec(&body).map_err(|source| FamilyError::Encode { source })?,
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
