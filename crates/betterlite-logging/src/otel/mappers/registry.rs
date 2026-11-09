use betterlite_config::loader::EnvironmentLookup;
use std::collections::BTreeSet;

use super::base::{AttributeMap, AttributeMapper, SpanData};
use super::genai::GenAIMapper;
use super::langfuse::LangfuseMapper;
use super::langtrace::LangtraceMapper;
use super::legacy::LegacyMapper;
use super::openinference::OpenInferenceMapper;
use super::utils::tool_attr_budget;
use super::weave::WeaveMapper;

pub enum Mapper {
    Genai(GenAIMapper),
    Legacy(LegacyMapper),
    Langfuse(LangfuseMapper),
    Langtrace(LangtraceMapper),
    OpenInference(OpenInferenceMapper),
    Weave(WeaveMapper),
}

type ToolDefinitionFactory = fn(usize) -> Mapper;
type PlainFactory = fn() -> Mapper;

// config.mapper_names resolves against these two tables; only the first spells tools out per index
#[rustfmt::skip]
const TOOL_DEFINITION_MAPPERS: &[(&str, ToolDefinitionFactory)] = &[
    ("genai", |budget| Mapper::Genai(GenAIMapper { tool_attr_budget: budget })),
    ("legacy", |budget| Mapper::Legacy(LegacyMapper { tool_attr_budget: budget })),
    ("openinference", |budget| Mapper::OpenInference(OpenInferenceMapper { tool_attr_budget: budget })),
];

#[rustfmt::skip]
const PLAIN_MAPPERS: &[(&str, PlainFactory)] = &[
    ("langfuse", || Mapper::Langfuse(LangfuseMapper)),
    ("langtrace", || Mapper::Langtrace(LangtraceMapper)),
    ("weave", || Mapper::Weave(WeaveMapper)),
];

#[derive(Debug, thiserror::Error)]
pub enum MapperError {
    #[error("unknown mapper name {name}; known: {known}")]
    Unknown { name: String, known: String },
}

pub fn resolve_mappers(names: &[String]) -> Result<Vec<Mapper>, MapperError> {
    let vocabularies = names
        .iter()
        .filter(|name| tool_definition_factory(name).is_some())
        .map(String::as_str)
        .collect::<BTreeSet<_>>()
        .len();
    let budget = tool_attr_budget(vocabularies);
    let mut mappers = Vec::with_capacity(names.len());
    for name in names {
        let mapper = build_mapper(name, budget).ok_or_else(|| MapperError::Unknown {
            name: name.clone(),
            known: known_names(),
        })?;
        mappers.push(mapper);
    }
    Ok(mappers)
}

impl Mapper {
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Genai(_) => "genai",
            Self::Legacy(_) => "legacy",
            Self::OpenInference(_) => "openinference",
            Self::Langfuse(_) => "langfuse",
            Self::Langtrace(_) => "langtrace",
            Self::Weave(_) => "weave",
        }
    }

    #[must_use]
    pub const fn tool_attr_budget(&self) -> Option<usize> {
        match self {
            Self::Genai(mapper) => Some(mapper.tool_attr_budget),
            Self::Legacy(mapper) => Some(mapper.tool_attr_budget),
            Self::OpenInference(mapper) => Some(mapper.tool_attr_budget),
            Self::Langfuse(_) | Self::Langtrace(_) | Self::Weave(_) => None,
        }
    }
}

impl AttributeMapper for Mapper {
    fn map(&self, data: &SpanData<'_>, environment: &dyn EnvironmentLookup) -> AttributeMap {
        match self {
            Self::Genai(mapper) => mapper.map(data, environment),
            Self::Legacy(mapper) => mapper.map(data, environment),
            Self::OpenInference(mapper) => mapper.map(data, environment),
            Self::Langfuse(mapper) => mapper.map(data, environment),
            Self::Langtrace(mapper) => mapper.map(data, environment),
            Self::Weave(mapper) => mapper.map(data, environment),
        }
    }
}

fn tool_definition_factory(name: &str) -> Option<&'static ToolDefinitionFactory> {
    TOOL_DEFINITION_MAPPERS
        .iter()
        .find(|(known, _make)| *known == name)
        .map(|(_known, make)| make)
}

fn build_mapper(name: &str, budget: usize) -> Option<Mapper> {
    tool_definition_factory(name)
        .map(|make| make(budget))
        .or_else(|| {
            PLAIN_MAPPERS
                .iter()
                .find(|(known, _make)| *known == name)
                .map(|(_known, make)| make())
        })
}

fn known_names() -> String {
    let mut names: Vec<&str> = TOOL_DEFINITION_MAPPERS
        .iter()
        .map(|(name, _make)| *name)
        .chain(PLAIN_MAPPERS.iter().map(|(name, _make)| *name))
        .collect();
    names.sort_unstable();
    names.join(", ")
}

#[path = "registry/tests.rs"]
mod tests;
