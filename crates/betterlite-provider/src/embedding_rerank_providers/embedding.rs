use std::sync::Arc;

use betterlite_core::embedding_rerank_providers::{EmbeddingConfig, embedding_config};
use betterlite_openai::cohere_embedding::CohereEmbeddingError;
use betterlite_openai::cohere_embedding::request as cohere_request;
use betterlite_openai::embedding::{EmbeddingInput, EmbeddingRequest};
use betterlite_openai::openai_base_config::Object as BaseObject;
use betterlite_openai::voyage_embedding::{self, VoyageEmbeddingVariant};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::assets;
use betterlite_tokenizer::engine::tiktoken::{Encoding, TiktokenEngine};
use secrecy::ExposeSecret;

use super::Common;
use super::error::FamilyError;
use super::request::{call, text_headers};
use super::{Call, CohereEmbeddingDecode, Decode, EmbeddingRerankPrepared};

mod host;

pub fn prepare(
    common: &Common<'_>,
    request: &EmbeddingRequest,
) -> Result<EmbeddingRerankPrepared, FamilyError> {
    let config = embedding_config(common.provider, common.model).ok_or_else(|| {
        FamilyError::Unsupported {
            provider: common.provider.to_string(),
        }
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
        EmbeddingConfig::JinaAi => host::prepare(host::Kind::Jina, &leaf)?,
        EmbeddingConfig::Infinity => host::prepare(host::Kind::Infinity, &leaf)?,
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
    // `headers = headers or extra_headers` followed by `headers.update(extra_headers)`, and no globals
    let mut supplied = leaf
        .common
        .headers
        .filter(|headers| !headers.is_empty())
        .map_or_else(|| leaf.common.extra_headers.clone(), Clone::clone);
    supplied.extend(leaf.common.extra_headers.clone());
    let headers = cohere_request::validate_environment(supplied, key.as_ref());
    let endpoint = cohere_request::get_complete_url(leaf.common.api_base);
    let mapped = cohere_request::map_openai_params(leaf.non_default, Object::new());
    let body = cohere_request::transform_embedding_request(
        leaf.common.model,
        &leaf.request.input,
        &mapped,
    )
    .map_err(FamilyError::from)?;
    // the Cohere leaf counts the input, so its tokens and token lists have no text to count
    let input = match &leaf.request.input {
        EmbeddingInput::Text(text) => vec![text.clone()],
        EmbeddingInput::Texts(texts) => texts.clone(),
        EmbeddingInput::Tokens(_) | EmbeddingInput::TokenLists(_) => {
            return Err(FamilyError::from(CohereEmbeddingError::InputNotText));
        }
    };
    let decode = Decode::CohereEmbedding(Box::new(CohereEmbeddingDecode {
        model: leaf.common.model.to_owned(),
        api_key: key,
        request_data: body.clone(),
        input,
        encoding: encoding()?,
    }));
    call(leaf.common, ("cohere", &endpoint), &headers, &body, decode)
}

fn voyage(leaf: &Leaf<'_, '_>) -> Result<Call, FamilyError> {
    let variant = VoyageEmbeddingVariant::for_model(leaf.common.model);
    let options = voyage_options(leaf.non_default, variant)?;
    let body: BaseObject = serde_json::from_value(voyage_embedding::request(
        leaf.common.model,
        serde_json::to_value(&leaf.request.input)?,
        &options,
        variant,
    ))?;
    // LiteLLM's Voyage handler receives no caller headers, and its validate_environment clears them
    let headers = text_headers(voyage_embedding::voyage_headers(
        BaseObject::new(),
        voyage_embedding::voyage_credentials(
            leaf.common.api_key.map(ExposeSecret::expose_secret),
            |name| leaf.common.environment.lookup(name),
        ),
    ));
    let endpoint = voyage_embedding::endpoint(leaf.common.api_base, variant);
    call(
        leaf.common,
        ("voyage", &endpoint),
        &headers,
        &body,
        Decode::VoyageEmbedding(variant),
    )
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
