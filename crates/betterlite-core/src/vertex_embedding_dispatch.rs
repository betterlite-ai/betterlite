use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::{
    ConfiguredCredentials, CredentialDecodeError, decode_vertex_credentials,
};
use betterlite_config::sections::vertex_embedding_dispatch::Settings as EmbeddingSettings;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::{Extra, Json};
use betterlite_models::lookup::info::ModelQuery;
use betterlite_models::lookup::resolve::find_info;
use betterlite_tokenizer::model_capabilities::BEDROCK_NAMES;
use thiserror::Error;

mod tests;

// main.py compares the model get_llm_provider already split from its provider slug, and so does the caller
pub const MULTIMODAL_EMBEDDING_MODELS: [&str; 2] =
    ["multimodalembedding", "multimodalembedding@001"];
const PROVIDER: &str = "vertex_ai";
const PROJECT_ALIASES: [&str; 2] = ["vertex_project", "vertex_ai_project"];
const LOCATION_ALIASES: [&str; 2] = ["vertex_location", "vertex_ai_location"];
const CREDENTIAL_ALIASES: [&str; 2] = ["vertex_credentials", "vertex_ai_credentials"];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Native {
    EmbedContent,
    Multimodal,
    Text,
}

// the two settings snapshots the environment and litellm_settings fallbacks come from
#[derive(Clone, Copy)]
pub struct Defaults<'settings> {
    pub vertex: &'settings VertexSettings,
    pub embedding: &'settings EmbeddingSettings,
}

// a deployment's litellm_params decoded once, so a pinned credential never travels as plain JSON
#[derive(Clone, Debug, Default)]
pub struct Pinned {
    pub project: Option<String>,
    pub location: Option<String>,
    pub credentials: Option<ConfiguredCredentials>,
    pub api_base: Option<String>,
}

#[derive(Clone, Copy)]
pub struct SelectionRequest<'call> {
    pub model: &'call str,
    pub api_base: Option<&'call str>,
    pub optional_params: &'call Extra,
    pub pinned: Option<&'call Pinned>,
}

#[derive(Clone, Debug)]
pub struct Resolution {
    pub native: Native,
    pub project: Option<String>,
    pub location: Option<String>,
    pub credentials: Option<ConfiguredCredentials>,
    pub api_base: Option<String>,
    // what the aliases were popped from, so the handler receives the params LiteLLM leaves it
    pub optional_params: Extra,
}

#[derive(Debug, Error)]
pub enum VertexEmbeddingDispatchError {
    #[error("the {field} option must be a string")]
    TextOption { field: String },
    #[error("the {field} option must be a string or an object")]
    CredentialType { field: String },
    #[error("the {field} option is not a readable credential")]
    Credentials {
        field: String,
        #[source]
        source: CredentialDecodeError,
    },
}

pub fn pin(params: &Extra) -> Result<Pinned, VertexEmbeddingDispatchError> {
    let mut remaining = params.clone();
    Ok(Pinned {
        project: take_alias(&mut remaining, &PROJECT_ALIASES)
            .map(text_option)
            .transpose()?,
        location: take_alias(&mut remaining, &LOCATION_ALIASES)
            .map(text_option)
            .transpose()?,
        credentials: take_alias(&mut remaining, &CREDENTIAL_ALIASES)
            .map(credential_option)
            .transpose()?,
        api_base: take_alias(&mut remaining, &["api_base"])
            .map(text_option)
            .transpose()?,
    })
}

pub fn select(
    request: SelectionRequest<'_>,
    defaults: Defaults<'_>,
    catalog: &ModelCatalog,
) -> Result<Resolution, VertexEmbeddingDispatchError> {
    let pinned = request.pinned.cloned().unwrap_or_default();
    let mut optional_params = request.optional_params.clone();
    let project = take_alias(&mut optional_params, &PROJECT_ALIASES)
        .map(text_option)
        .transpose()?
        .or(pinned.project)
        .or_else(|| defaults.vertex.project.clone())
        .or_else(|| defaults.embedding.project.clone());
    let location = take_alias(&mut optional_params, &LOCATION_ALIASES)
        .map(text_option)
        .transpose()?
        .or(pinned.location)
        .or_else(|| defaults.vertex.location.clone());
    let credentials = take_alias(&mut optional_params, &CREDENTIAL_ALIASES)
        .map(credential_option)
        .transpose()?
        .or(pinned.credentials)
        .or_else(|| defaults.vertex.credentials.clone())
        .or_else(|| defaults.embedding.credentials.clone());
    let api_base = request
        .api_base
        .map(str::to_owned)
        .or(pinned.api_base)
        .or_else(|| defaults.embedding.api_base.clone());
    Ok(Resolution {
        native: native_api(catalog, request.model, &optional_params),
        project,
        location,
        credentials,
        api_base,
        optional_params,
    })
}

fn native_api(catalog: &ModelCatalog, model: &str, optional_params: &Extra) -> Native {
    if uses_embed_content(catalog, model) {
        return Native::EmbedContent;
    }
    if optional_params.contains_key("image")
        || optional_params.contains_key("video")
        || MULTIMODAL_EMBEDDING_MODELS.contains(&model)
    {
        return Native::Multimodal;
    }
    Native::Text
}

// get_model_info's exception and its missing entry both leave the flag false without a warning
fn uses_embed_content(catalog: &ModelCatalog, model: &str) -> bool {
    let query = ModelQuery {
        model,
        provider: Some(PROVIDER),
    };
    let Ok(Some(resolved)) = find_info(catalog, &BEDROCK_NAMES, &query) else {
        return false;
    };
    catalog
        .lookup
        .get(resolved.key.as_str())
        .and_then(|canonical| catalog.entries.get(canonical.as_ref()))
        .is_some_and(|entry| entry.fields.capabilities.media.uses_embed_content == Some(true))
}

// an absent or null alias falls through to the next name; a present one stays and stops the chain
fn take_alias(params: &mut Extra, names: &[&'static str]) -> Option<(&'static str, Json)> {
    for name in names {
        let Some(value) = params.shift_remove(*name) else {
            continue;
        };
        if value == Json::Null {
            continue;
        }
        return Some((*name, value));
    }
    None
}

fn text_option(
    (field, value): (&'static str, Json),
) -> Result<String, VertexEmbeddingDispatchError> {
    let Json::Text(text) = value else {
        return Err(VertexEmbeddingDispatchError::TextOption {
            field: field.to_owned(),
        });
    };
    Ok(text)
}

// LiteLLM declares VERTEX_CREDENTIALS_TYPES as a string or an inline service-account object
fn credential_option(
    (field, value): (&'static str, Json),
) -> Result<ConfiguredCredentials, VertexEmbeddingDispatchError> {
    let decoded = match value {
        Json::Text(text) => ConfiguredCredentials::from_text(text),
        Json::Object(account) => serde_json::to_vec(&account)
            .map_err(CredentialDecodeError::Json)
            .and_then(|document| decode_vertex_credentials(&document))
            .map(ConfiguredCredentials::Source),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {
            return Err(VertexEmbeddingDispatchError::CredentialType {
                field: field.to_owned(),
            });
        }
    };
    decoded.map_err(|source| VertexEmbeddingDispatchError::Credentials {
        field: field.to_owned(),
        source,
    })
}
