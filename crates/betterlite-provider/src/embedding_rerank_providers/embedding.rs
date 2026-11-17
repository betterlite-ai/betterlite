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
use super::request::{call, host_options, text_headers};
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
    let leaf = Leaf {
        common,
        request,
        non_default: &non_default,
    };
    let call = match config {
        EmbeddingConfig::Cohere => cohere(&leaf)?,
        EmbeddingConfig::Voyage
        | EmbeddingConfig::VoyageContextual
        | EmbeddingConfig::VoyageMultimodal => voyage(&leaf)?,
        EmbeddingConfig::JinaAi => host(HostKind::Jina, &leaf)?,
        EmbeddingConfig::Infinity => host(HostKind::Infinity, &leaf)?,
    };
    Ok(EmbeddingRerankPrepared(call))
}

// the caller's request and the parameters it set, which every family's steps read
struct Leaf<'leaf, 'request> {
    common: &'leaf Common<'request>,
    request: &'leaf EmbeddingRequest,
    non_default: &'leaf Object,
}

fn cohere(leaf: &Leaf<'_, '_>) -> Result<Call, FamilyError> {
    let key = leaf.common.settings.resolve_api_key(leaf.common.api_key);
    let headers = cohere_request::validate_environment(
        merged_headers(leaf.common.headers, leaf.common.extra_headers),
        key.as_ref(),
    );
    let endpoint = cohere_request::get_complete_url(leaf.common.api_base);
    let mapped = cohere_request::map_openai_params(leaf.non_default, Object::new());
    let body = cohere_request::transform_embedding_request(
        leaf.common.model,
        &leaf.request.input,
        &mapped,
    )
    .map_err(FamilyError::from)?;
    let decode = Decode::CohereEmbedding(Box::new(CohereEmbeddingDecode {
        model: leaf.common.model.to_owned(),
        api_key: key,
        request_data: body.clone(),
        input: texts(&leaf.request.input)?,
        encoding: encoding()?,
    }));
    call(leaf.common, ("cohere", endpoint), &headers, &body, decode)
}

fn voyage(leaf: &Leaf<'_, '_>) -> Result<Call, FamilyError> {
    let (body, variant) = voyage_body(leaf)?;
    let headers = voyage_headers(leaf.common);
    let endpoint = voyage_embedding::endpoint(leaf.common.api_base, variant);
    call(
        leaf.common,
        ("voyage", endpoint),
        &headers,
        &body,
        Decode::VoyageEmbedding(variant),
    )
}

fn voyage_body(leaf: &Leaf<'_, '_>) -> Result<(BaseObject, VoyageEmbeddingVariant), FamilyError> {
    let variant = VoyageEmbeddingVariant::for_model(leaf.common.model);
    let options = voyage_options(leaf.non_default, variant)?;
    let given = serde_json::to_value(&leaf.request.input)?;
    let body = voyage_embedding::request(leaf.common.model, given, &options, variant);
    Ok((serde_json::from_value(body)?, variant))
}

// LiteLLM's Voyage handler receives no caller headers, and its validate_environment clears them
fn voyage_headers(common: &Common<'_>) -> Headers {
    let authorization = voyage_embedding::voyage_credentials(
        common.api_key.map(ExposeSecret::expose_secret),
        |name| common.environment.lookup(name),
    );
    text_headers(voyage_embedding::voyage_headers(
        BaseObject::new(),
        authorization,
    ))
}

// LiteLLM's Voyage builder keeps only the caller's parameters its variant declares
fn voyage_options(
    non_default: &Object,
    variant: VoyageEmbeddingVariant,
) -> Result<BaseObject, FamilyError> {
    let supported: &[&str] = match variant {
        VoyageEmbeddingVariant::Multimodal => &["dimensions"],
        VoyageEmbeddingVariant::Text | VoyageEmbeddingVariant::Contextual => {
            &["encoding_format", "dimensions"]
        }
    };
    let supplied: BaseObject = serde_json::from_value(serde_json::to_value(non_default)?)?;
    Ok(BaseObject::from_iter(supported.iter().filter_map(|name| {
        supplied
            .get(*name)
            .map(|value| ((*name).to_owned(), value.clone()))
    })))
}

// the two host leaves share LiteLLM's shape, and only their credentials, endpoint and reply differ
fn host(kind: HostKind, leaf: &Leaf<'_, '_>) -> Result<Call, FamilyError> {
    match kind {
        HostKind::Jina => {
            let credentials = jina_embedding::resolve_credentials(
                leaf.common.api_base,
                leaf.common.api_key,
                leaf.common.host,
            );
            let endpoint = jina_embedding::get_complete_url(Some(credentials.api_base.as_str()));
            let mapped = host_options(leaf.non_default, jina_embedding::map_openai_params)?;
            let given = serde_json::to_value(&leaf.request.input)?;
            let body =
                jina_embedding::transform_embedding_request(leaf.common.model, &given, &mapped);
            let headers =
                jina_embedding::validate_environment(&Headers::new(), credentials.api_key.as_ref());
            let decode = Decode::JinaEmbedding(Box::new(JinaEmbeddingDecode {
                api_key: credentials.api_key,
                request_data: body.clone(),
            }));
            call(leaf.common, ("jina", endpoint), &headers, &body, decode)
        }
        HostKind::Infinity => {
            let endpoint = infinity_embedding::get_complete_url(leaf.common.api_base)?;
            let mapped = host_options(leaf.non_default, infinity_embedding::map_openai_params)?;
            let given = serde_json::to_value(&leaf.request.input)?;
            let body =
                infinity_embedding::transform_embedding_request(leaf.common.model, &given, &mapped);
            let headers = infinity_embedding::validate_environment(
                &Headers::new(),
                leaf.common.api_key,
                leaf.common.host,
            );
            call(
                leaf.common,
                ("infinity", endpoint),
                &headers,
                &body,
                Decode::InfinityEmbedding,
            )
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum HostKind {
    Jina,
    Infinity,
}

// the Cohere leaf counts the input, so its tokens and token lists have no text to count
fn texts(input: &EmbeddingInput) -> Result<Vec<String>, FamilyError> {
    match input {
        EmbeddingInput::Text(text) => Ok(vec![text.clone()]),
        EmbeddingInput::Texts(texts) => Ok(texts.clone()),
        EmbeddingInput::Tokens(_) | EmbeddingInput::TokenLists(_) => {
            Err(FamilyError::from(CohereEmbeddingError::InputNotText))
        }
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

// LiteLLM counts the input with litellm.encoding when Cohere bills neither input nor image tokens
fn encoding() -> Result<Engine, FamilyError> {
    let vocabulary = assets::bundled(Encoding::Cl100kBase).ok_or(FamilyError::Vocabulary)?;
    let engine = TiktokenEngine::from_bpe_file(Encoding::Cl100kBase, &vocabulary)
        .map_err(CohereEmbeddingError::Encoding)?;
    Ok(Engine::Tiktoken(Arc::new(engine)))
}

// LiteLLM's get_optional_params_embeddings receives only the parameters the caller set
fn non_default_params(request: &EmbeddingRequest) -> Result<Object, FamilyError> {
    let mut fields: Object = serde_json::from_value(serde_json::to_value(request)?)?;
    fields.shift_remove("model");
    fields.shift_remove("input");
    fields.retain(|_, value| !matches!(value, Json::Null));
    Ok(fields)
}
