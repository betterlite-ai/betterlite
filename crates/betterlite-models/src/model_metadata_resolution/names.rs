use std::collections::{BTreeMap, BTreeSet};

use indexmap::IndexMap;
use serde_json::value::RawValue;

use crate::identity::provider_id::ProviderId;
use crate::model_catalog_keys::{PotentialModelNames, strip_model_name};
use crate::provider_metadata_factory::selection::select;
use crate::resolution::{ResolutionContext, ResolutionRequest, resolve};

const MODEL_COST_MAP_LIMIT: usize = 16 * 1024 * 1024;
const VERTEX_NAMESPACE: &str = "vertex_ai/";
const LLAMA_PROVIDER: &str = "vertex_ai-llama_models";
const MISTRAL_PROVIDER: &str = "vertex_ai-mistral_models";
const AI21_PROVIDER: &str = "vertex_ai-ai21_models";

// litellm.azure_llms then litellm.azure_embedding_models, so an embedding alias wins where both name a model.
const AZURE_ALIASES: [(&str, &str); 7] = [
    ("gpt-35-turbo", "azure/gpt-35-turbo"),
    ("gpt-35-turbo-16k", "azure/gpt-35-turbo-16k"),
    ("gpt-35-turbo-instruct", "azure/gpt-35-turbo-instruct"),
    ("azure/gpt-41", "gpt-4.1"),
    ("azure/gpt-41-mini", "gpt-4.1-mini"),
    ("azure/gpt-41-nano", "gpt-4.1-nano"),
    ("ada", "azure/ada"),
];

#[derive(Debug, thiserror::Error)]
pub enum ModelCostMapError {
    #[error("model cost map is {size} bytes, over the {limit} byte limit")]
    TooLarge { size: usize, limit: usize },
    #[error("model cost map JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug)]
pub struct ModelCostMap {
    pub entries: IndexMap<String, Box<RawValue>>,
    pub vertex: VertexModelSets,
}

impl ModelCostMap {
    pub fn parse(bytes: &[u8]) -> Result<Self, ModelCostMapError> {
        if bytes.len() > MODEL_COST_MAP_LIMIT {
            return Err(ModelCostMapError::TooLarge {
                size: bytes.len(),
                limit: MODEL_COST_MAP_LIMIT,
            });
        }
        let entries: IndexMap<String, Box<RawValue>> = serde_json::from_slice(bytes)?;
        let vertex = VertexModelSets::build(&entries);
        Ok(Self { entries, vertex })
    }
}

// LiteLLM fills these three sets from the cost map once; the names lose the vertex_ai/ namespace.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct VertexModelSets {
    pub llama: BTreeSet<String>,
    pub mistral: BTreeSet<String>,
    pub ai21: BTreeSet<String>,
}

impl VertexModelSets {
    #[must_use]
    pub fn build(entries: &IndexMap<String, Box<RawValue>>) -> Self {
        let mut sets = Self::default();
        for (key, entry) in entries {
            let Ok(fields) = serde_json::from_str::<BTreeMap<String, Box<RawValue>>>(entry.get())
            else {
                continue;
            };
            let Some(provider) = fields
                .get("litellm_provider")
                .and_then(|value| serde_json::from_str::<String>(value.get()).ok())
            else {
                continue;
            };
            let name = key.replace(VERTEX_NAMESPACE, "");
            match provider.as_str() {
                LLAMA_PROVIDER => sets.llama.insert(name),
                MISTRAL_PROVIDER => sets.mistral.insert(name),
                AI21_PROVIDER => sets.ai21.insert(name),
                _ => false,
            };
        }
        sets
    }
}

// The bedrock and mantle helpers live in crates above this one, so the caller passes them in.
#[derive(Clone, Copy)]
pub struct NameNormalizers {
    pub bedrock_base_model: fn(&str) -> String,
    pub bedrock_routing_prefix: fn(&str) -> String,
    pub mantle_region_split: for<'model> fn(&'model str) -> (Option<&'model str>, &'model str),
}

pub struct PotentialNamesInput<'input> {
    pub model: &'input str,
    pub custom_llm_provider: Option<&'input str>,
    pub resolution: &'input ResolutionContext<'input>,
    pub normalizers: NameNormalizers,
}

#[must_use]
pub fn potential_model_names(input: &PotentialNamesInput<'_>) -> PotentialModelNames {
    let base = base_names(input);
    let normalizers = input.normalizers;
    let mut split_model = base.split_model;
    if matches!(
        base.provider.as_deref(),
        Some("bedrock" | "bedrock_converse")
    ) {
        split_model = (normalizers.bedrock_routing_prefix)(&split_model);
    }
    let region_free_split_model = if base.provider.as_deref() == Some("bedrock_mantle") {
        (normalizers.mantle_region_split)(&split_model).1.to_owned()
    } else {
        split_model
    };
    let combined_stripped_model_name = if base.provider.as_deref() == Some("bedrock_mantle") {
        format!(
            "bedrock_mantle/{}",
            strip_model_name(
                &region_free_split_model,
                base.provider.as_deref(),
                normalizers.bedrock_base_model
            )
        )
    } else {
        base.combined_stripped_model_name
    };
    PotentialModelNames {
        provider_prefixed_model_name: cost_key(&region_free_split_model, base.provider.as_deref())
            .unwrap_or(base.provider_prefixed_model_name),
        split_model: region_free_split_model,
        combined_model_name: base.combined_model_name,
        stripped_model_name: base.stripped_model_name,
        combined_stripped_model_name,
        custom_llm_provider: base.provider,
    }
}

struct BaseNames {
    split_model: String,
    provider: Option<String>,
    combined_model_name: String,
    stripped_model_name: String,
    combined_stripped_model_name: String,
    provider_prefixed_model_name: String,
}

fn base_names(input: &PotentialNamesInput<'_>) -> BaseNames {
    input.custom_llm_provider.map_or_else(
        || inferred_names(input),
        |provider| explicit_names(input, provider),
    )
}

fn inferred_names(input: &PotentialNamesInput<'_>) -> BaseNames {
    let (split_model, provider) = match inferred_provider(input.model, input.resolution) {
        Some((model, provider)) => (model, Some(provider)),
        None => (input.model.to_owned(), None),
    };
    let stripped_model_name = strip_model_name(
        input.model,
        provider.as_deref(),
        input.normalizers.bedrock_base_model,
    );
    BaseNames {
        split_model,
        provider,
        combined_model_name: input.model.to_owned(),
        combined_stripped_model_name: stripped_model_name.clone(),
        stripped_model_name,
        provider_prefixed_model_name: input.model.to_owned(),
    }
}

fn explicit_names(input: &PotentialNamesInput<'_>, provider: &str) -> BaseNames {
    let prefixed = input.model.strip_prefix(&format!("{provider}/"));
    let split_model = prefixed.unwrap_or(input.model);
    let stripped_model_name = strip_model_name(
        split_model,
        Some(provider),
        input.normalizers.bedrock_base_model,
    );
    let combined_model_name = prefixed.map_or_else(
        || format!("{provider}/{}", input.model),
        |_| input.model.to_owned(),
    );
    BaseNames {
        combined_model_name,
        combined_stripped_model_name: format!("{provider}/{stripped_model_name}"),
        stripped_model_name,
        split_model: split_model.to_owned(),
        provider: Some(provider.to_owned()),
        provider_prefixed_model_name: format!("{provider}/{}", input.model),
    }
}

fn inferred_provider(model: &str, context: &ResolutionContext<'_>) -> Option<(String, String)> {
    let request = ResolutionRequest {
        model,
        custom_llm_provider: None,
        api_base: None,
        api_key: None,
    };
    resolve(&request, context)
        .ok()
        .map(|resolved| (resolved.model, resolved.provider.slug().to_owned()))
}

fn cost_key(model: &str, provider: Option<&str>) -> Option<String> {
    let parsed = provider?.parse::<ProviderId>().ok()?;
    let adapter = select(Some(model), parsed)?;
    adapter
        .get_model_cost_key(model)
        .filter(|key| !key.is_empty())
}

pub fn apply_aliases(model: &mut String, provider: &mut Option<String>, vertex: &VertexModelSets) {
    if let Some((_, aliased)) = AZURE_ALIASES
        .iter()
        .find(|(name, _)| *name == model.as_str())
    {
        *model = (*aliased).to_owned();
    }
    if provider.as_deref() == Some("vertex_ai_beta") {
        *provider = Some("vertex_ai".to_owned());
    }
    if provider.as_deref() == Some("vertex_ai") {
        let meta = format!("meta/{model}");
        if vertex.llama.contains(&meta) {
            *model = meta;
        } else {
            let latest = format!("{model}@latest");
            if vertex.mistral.contains(&latest) || vertex.ai21.contains(&latest) {
                *model = latest;
            }
        }
    }
}
