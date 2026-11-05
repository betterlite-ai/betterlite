use std::collections::HashMap;

use indexmap::IndexMap;
use unicode_general_category::{GeneralCategory, get_general_category};

mod tests;

const DECIMAL_DIGIT: fn(char) -> bool =
    |character| get_general_category(character) == GeneralCategory::DecimalNumber;
const DATED_SNAPSHOT_SHAPE: &str = "-0000-00-00";

#[derive(Debug, thiserror::Error)]
pub enum ModelCatalogKeysError {
    #[error("model {key} is not in the model cost map")]
    MissingModel { key: String },
    #[error("litellm_provider of the catalog entry is not a string")]
    ProviderNotText,
}

pub trait ClearableCache {
    fn clear(&mut self);
}

impl<Key, Value, State> ClearableCache for HashMap<Key, Value, State> {
    fn clear(&mut self) {
        Self::clear(self);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EntryProvider<'entry> {
    Unconstrained,
    Text(&'entry str),
    NotText,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Deserialize)]
pub struct PotentialModelNames {
    pub split_model: String,
    pub combined_model_name: String,
    pub stripped_model_name: String,
    pub combined_stripped_model_name: String,
    pub provider_prefixed_model_name: String,
    pub custom_llm_provider: Option<String>,
}

#[derive(Debug, Default)]
pub struct ModelCostKeys {
    pub lowercase_index: Option<HashMap<String, String>>,
    pub generation: u64,
}

#[must_use]
pub fn strip_stable_vertex_version(model: &str) -> String {
    let body = model.strip_suffix('\n').unwrap_or(model);
    let name = body
        .trim_end_matches(DECIMAL_DIGIT)
        .strip_suffix('-')
        .filter(|stem| stem.len() < body.len().saturating_sub(1));
    name.map_or_else(
        || model.to_owned(),
        |stem| format!("{stem}{}", model.get(body.len()..).unwrap_or_default()),
    )
}

#[must_use]
pub fn strip_dated_snapshot_suffix(model: &str) -> String {
    let (body, newline) = model
        .strip_suffix('\n')
        .map_or((model, ""), |body| (body, "\n"));
    let suffix_start = body
        .char_indices()
        .rev()
        .nth(DATED_SNAPSHOT_SHAPE.len().saturating_sub(1))
        .and_then(|(index, _)| body.split_at_checked(index));
    match suffix_start {
        Some((name, suffix))
            if suffix.chars().zip(DATED_SNAPSHOT_SHAPE.chars()).all(
                |(actual, shape)| match shape {
                    '0' => DECIMAL_DIGIT(actual),
                    _ => actual == shape,
                },
            ) =>
        {
            format!("{name}{newline}")
        }
        _ => model.to_owned(),
    }
}

#[must_use]
pub fn strip_finetune_model_name(model: &str) -> String {
    model
        .rmatch_indices(':')
        .nth(2)
        .and_then(|(index, _)| model.get(..index))
        .unwrap_or(model)
        .to_owned()
}

// B2203's bedrock normalizer lives in betterlite-context, which depends on this crate, so the caller passes it in
#[must_use]
pub fn strip_model_name(
    model: &str,
    custom_llm_provider: Option<&str>,
    bedrock_base_model: fn(&str) -> String,
) -> String {
    match custom_llm_provider {
        Some("bedrock" | "bedrock_converse") => bedrock_base_model(model),
        Some("vertex_ai" | "gemini" | "databricks") => strip_stable_vertex_version(model),
        _ if model.contains("ft:") => strip_finetune_model_name(model),
        _ => strip_dated_snapshot_suffix(model),
    }
}

pub fn check_provider_match(
    entry_provider: EntryProvider<'_>,
    custom_llm_provider: Option<&str>,
) -> Result<bool, ModelCatalogKeysError> {
    let Some(requested) = custom_llm_provider else {
        return Ok(true);
    };
    match entry_provider {
        EntryProvider::Unconstrained => return Ok(true),
        EntryProvider::Text(entry) if entry == requested => return Ok(true),
        EntryProvider::Text(_) | EntryProvider::NotText => {}
    }
    let prefix_family = match requested {
        "vertex_ai" | "fireworks_ai" => Some(requested),
        _ if requested.starts_with("bedrock") => Some("bedrock"),
        _ => None,
    };
    if let Some(family) = prefix_family {
        match entry_provider {
            EntryProvider::Text(entry) if entry.starts_with(family) => return Ok(true),
            EntryProvider::NotText => return Err(ModelCatalogKeysError::ProviderNotText),
            EntryProvider::Text(_) | EntryProvider::Unconstrained => {}
        }
    }
    Ok(match requested {
        "litellm_proxy" | "github" | "nadir" => true,
        "azure_ai" => matches!(entry_provider, EntryProvider::Text("azure" | "openai")),
        _ => false,
    })
}

impl ModelCostKeys {
    pub fn invalidate<Full, Helper>(
        &mut self,
        full_model_info: &mut Full,
        helper_model_info: &mut Helper,
    ) where
        Full: ClearableCache,
        Helper: ClearableCache,
    {
        self.lowercase_index = None;
        self.generation = self.generation.saturating_add(1);
        full_model_info.clear();
        helper_model_info.clear();
    }

    pub fn get_model_cost_key<Entry>(
        &mut self,
        model_cost: &IndexMap<String, Entry>,
        potential_key: &str,
    ) -> Option<String> {
        if model_cost.contains_key(potential_key) {
            return Some(potential_key.to_owned());
        }
        let lowercase_key = potential_key.to_lowercase();
        let matched_key = self
            .lowercase_index
            .get_or_insert_with(|| {
                model_cost
                    .keys()
                    .map(|key| (key.to_lowercase(), key.clone()))
                    .collect()
            })
            .get(&lowercase_key)?;
        if model_cost.contains_key(matched_key) {
            return Some(matched_key.clone());
        }
        let rebuilt = self.lowercase_index.insert(
            model_cost
                .keys()
                .map(|key| (key.to_lowercase(), key.clone()))
                .collect(),
        );
        rebuilt
            .get(&lowercase_key)
            .filter(|key| model_cost.contains_key(*key))
            .cloned()
    }

    pub fn has_potential_model_name<Entry>(
        &mut self,
        model_cost: &IndexMap<String, Entry>,
        names: &PotentialModelNames,
    ) -> bool {
        let values = [
            names.split_model.as_str(),
            names.combined_model_name.as_str(),
            names.stripped_model_name.as_str(),
            names.combined_stripped_model_name.as_str(),
            names.provider_prefixed_model_name.as_str(),
            names.custom_llm_provider.as_deref().unwrap_or("None"),
        ];
        values
            .iter()
            .any(|value| self.get_model_cost_key(model_cost, value).is_some())
    }
}

pub fn get_model_info_from_model_cost<'catalog, Entry>(
    model_cost: &'catalog IndexMap<String, Entry>,
    key: &str,
) -> Result<&'catalog Entry, ModelCatalogKeysError> {
    model_cost
        .get(key)
        .ok_or_else(|| ModelCatalogKeysError::MissingModel {
            key: key.to_owned(),
        })
}
