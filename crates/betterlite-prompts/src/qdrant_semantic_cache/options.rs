use std::collections::BTreeMap;

use betterlite_models::provider_defaults_a::environment::EnvironmentLookup;
use secrecy::{ExposeSecret as _, SecretString};

use super::error::QdrantSemanticError;

pub const CACHE_KEY_FIELD_NAME: &str = "litellm_cache_key";
pub const DEFAULT_VECTOR_SIZE: u32 = 1536;
pub const DEFAULT_SCALAR_QUANTILE: f64 = 0.99;
pub const DEFAULT_EMBEDDING_MODEL: &str = "text-embedding-ada-002";
pub const DEFAULT_EMBEDDING_TIMEOUT: f64 = 5.0;

const BASE_ENVIRONMENT_NAMES: [&str; 2] = ["QDRANT_URL", "QDRANT_API_BASE"];
const API_KEY_ENVIRONMENT_NAME: &str = "QDRANT_API_KEY";
const ENVIRONMENT_PREFIX: &str = "os.environ/";

#[derive(Debug, Default)]
pub struct QdrantSemanticOptions {
    pub collection_name: Option<String>,
    pub similarity_threshold: Option<f64>,
    pub qdrant_api_base: Option<String>,
    pub qdrant_api_key: Option<SecretString>,
    pub quantization_config: Option<String>,
    pub embedding_model: Option<String>,
    pub vector_size: Option<u32>,
    pub scalar_quantile: Option<f64>,
    pub embedding_max_input_tokens: Option<i64>,
    pub embedding_timeout: Option<f64>,
}

// The values the config loader resolved from QDRANT_* before the cache is built.
#[derive(Debug, Default)]
pub struct QdrantEnv {
    pub values: BTreeMap<String, String>,
    pub vector_size: Option<u32>,
    pub scalar_quantile: Option<f64>,
}

impl EnvironmentLookup for QdrantEnv {
    fn lookup(&self, name: &str) -> Option<String> {
        self.values.get(name).cloned()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Quantization {
    Binary,
    Scalar,
    Product,
}

impl Quantization {
    pub fn parse(configured: Option<&str>) -> Result<Self, QdrantSemanticError> {
        match configured {
            None | Some("binary") => Ok(Self::Binary),
            Some("scalar") => Ok(Self::Scalar),
            Some("product") => Ok(Self::Product),
            Some(_) => Err(QdrantSemanticError::InvalidQuantization),
        }
    }
}

pub(crate) fn resolve_base(
    configured: Option<String>,
    env: &dyn EnvironmentLookup,
) -> Result<String, QdrantSemanticError> {
    let explicit = match configured {
        Some(value) => resolve_reference(&value, "qdrant_api_base", env)?,
        None => None,
    };
    explicit
        .or_else(|| {
            BASE_ENVIRONMENT_NAMES
                .iter()
                .find_map(|name| env.lookup(name).filter(|value| !value.is_empty()))
        })
        .ok_or(QdrantSemanticError::MissingUrl)
}

pub(crate) fn resolve_key(
    configured: Option<SecretString>,
    env: &dyn EnvironmentLookup,
) -> Result<Option<SecretString>, QdrantSemanticError> {
    let explicit = match configured {
        Some(secret) => resolve_reference(secret.expose_secret(), "qdrant_api_key", env)?
            .map(SecretString::from),
        None => None,
    };
    Ok(explicit.or_else(|| {
        env.lookup(API_KEY_ENVIRONMENT_NAME)
            .filter(|value| !value.is_empty())
            .map(SecretString::from)
    }))
}

// get_secret_str reads an os.environ/ reference and treats a variable that resolves empty as unset.
fn resolve_reference(
    value: &str,
    path: &'static str,
    env: &dyn EnvironmentLookup,
) -> Result<Option<String>, QdrantSemanticError> {
    let Some(name) = value.strip_prefix(ENVIRONMENT_PREFIX) else {
        return Ok(Some(value.to_owned()));
    };
    let resolved = env
        .lookup(name)
        .ok_or_else(|| QdrantSemanticError::MissingEnvironment {
            variable: name.to_owned(),
            path,
        })?;
    Ok((!resolved.is_empty()).then_some(resolved))
}
