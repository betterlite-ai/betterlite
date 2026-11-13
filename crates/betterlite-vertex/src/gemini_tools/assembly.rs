use std::collections::BTreeMap;
use std::mem;

use betterlite_config::sections::vertex_schema_shaping::Settings;
use betterlite_prompts::arguments::json::{Json, Object};

use super::cleanup;
use super::error::GeminiToolsError;
use super::records::{FunctionDeclaration, RetrievalConfig, Tool, ToolConfig};
use super::specials;
use super::tool_lookup::{tool_value, underscore_name};
use crate::vertex_schema::build::{SchemaFormat, shape_parameters};

#[derive(Debug)]
pub struct MappedTools {
    pub tools: Vec<Tool>,
    pub tool_config: Option<ToolConfig>,
    pub warnings: usize,
}
pub fn map_tools(
    tools: &[Object],
    include_server_side: bool,
    tool_config: Option<ToolConfig>,
    settings: &Settings,
) -> Result<MappedTools, GeminiToolsError> {
    let mut collected = Collected::default();
    for tool in tools {
        let mut cleaned = tool.clone();
        cleanup::remove_unsupported_fields(&mut cleaned);
        collected.add(&cleaned, settings)?;
    }
    let warnings = collected
        .resolve_search_conflict(include_server_side)
        .saturating_add(collected.invalid);
    let mut config = tool_config;
    if let Some(retrieval) = collected.retrieval.take() {
        let merged = config.get_or_insert_with(ToolConfig::default);
        merged.retrieval_config = Some(retrieval);
    }
    Ok(MappedTools {
        tools: collected.assemble(),
        tool_config: config,
        warnings,
    })
}
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Slot {
    GoogleSearch,
    GoogleSearchRetrieval,
    EnterpriseWebSearch,
    CodeExecution,
    UrlContext,
    GoogleMaps,
    ComputerUse,
}
const SEARCH_SLOTS: [Slot; 4] = [
    Slot::GoogleSearch,
    Slot::GoogleSearchRetrieval,
    Slot::EnterpriseWebSearch,
    Slot::UrlContext,
];
const SPECIALS: [(&str, Slot); 7] = [
    ("codeExecution", Slot::CodeExecution),
    ("googleSearch", Slot::GoogleSearch),
    ("googleSearchRetrieval", Slot::GoogleSearchRetrieval),
    ("enterpriseWebSearch", Slot::EnterpriseWebSearch),
    ("urlContext", Slot::UrlContext),
    ("googleMaps", Slot::GoogleMaps),
    ("computerUse", Slot::ComputerUse),
];
#[derive(Default)]
struct Collected {
    declarations: Vec<FunctionDeclaration>,
    specials: BTreeMap<Slot, Object>,
    retrieval: Option<RetrievalConfig>,
    invalid: usize,
}
impl Collected {
    fn add(&mut self, tool: &Object, settings: &Settings) -> Result<(), GeminiToolsError> {
        let function_payload = function_object(tool)?;
        let transformed = specials::transform_type(tool);
        if let Some(name) = single_key(&transformed)
            && let Some((wire_name, slot)) = special_slot(name)
        {
            match special_value(&transformed, wire_name)? {
                Some(options) => self.record(slot, options),
                None if slot == Slot::ComputerUse => {
                    self.specials.insert(slot, Object::new());
                }
                None => {}
            }
            return Ok(());
        }
        match function_payload {
            Some(payload) => self.declarations.push(declaration(&payload, settings)?),
            None => self.invalid = self.invalid.saturating_add(1),
        }
        Ok(())
    }
    fn record(&mut self, slot: Slot, options: Object) {
        if slot == Slot::GoogleMaps {
            let (cleaned, retrieval) = specials::extract_maps(&options);
            self.retrieval = retrieval;
            self.specials.insert(slot, cleaned);
        } else if slot == Slot::ComputerUse {
            let transformed = specials::transform_computer_use(&options);
            self.specials.insert(slot, transformed);
        } else {
            self.specials.insert(slot, options);
        }
    }
    fn resolve_search_conflict(&mut self, include_server_side: bool) -> usize {
        let has_search = SEARCH_SLOTS
            .iter()
            .any(|slot| self.specials.contains_key(slot));
        if self.declarations.is_empty() || !has_search || include_server_side {
            return 0;
        }
        tracing::warn!(
            "function declarations cannot be mixed with search tools; dropping googleSearch, googleSearchRetrieval, enterpriseWebSearch and url_context"
        );
        for slot in SEARCH_SLOTS {
            self.specials.remove(&slot);
        }
        1
    }
    fn assemble(mut self) -> Vec<Tool> {
        let mut tools = Vec::new();
        if !self.declarations.is_empty() {
            tools.push(Tool::FunctionDeclarations {
                function_declarations: mem::take(&mut self.declarations),
            });
        }
        for (slot, options) in self.specials {
            tools.push(tool_for(slot, options));
        }
        tools
    }
}
const fn tool_for(slot: Slot, options: Object) -> Tool {
    match slot {
        Slot::GoogleSearch => Tool::GoogleSearch {
            google_search: options,
        },
        Slot::GoogleSearchRetrieval => Tool::GoogleSearchRetrieval {
            google_search_retrieval: options,
        },
        Slot::EnterpriseWebSearch => Tool::EnterpriseWebSearch {
            enterprise_web_search: options,
        },
        Slot::CodeExecution => Tool::CodeExecution {
            code_execution: options,
        },
        Slot::UrlContext => Tool::UrlContext {
            url_context: options,
        },
        Slot::GoogleMaps => Tool::GoogleMaps {
            google_maps: options,
        },
        Slot::ComputerUse => Tool::ComputerUse {
            computer_use: options,
        },
    }
}
fn single_key(tool: &Object) -> Option<&str> {
    let mut keys = tool.keys();
    let name = keys.next()?;
    if keys.next().is_some() {
        return None;
    }
    Some(name.as_str())
}
fn special_slot(name: &str) -> Option<(&'static str, Slot)> {
    SPECIALS
        .iter()
        .find(|(camel, _)| name == *camel || name == underscore_name(camel))
        .map(|(camel, slot)| (*camel, *slot))
}
fn function_object(tool: &Object) -> Result<Option<Object>, GeminiToolsError> {
    if let Some(function) = tool.get("function") {
        return function
            .as_object()
            .cloned()
            .map(Some)
            .ok_or(GeminiToolsError::FunctionField { field: "function" });
    }
    Ok(tool.contains_key("name").then(|| tool.clone()))
}
fn special_value(tool: &Object, name: &'static str) -> Result<Option<Object>, GeminiToolsError> {
    tool_value(tool, name).map_or(Ok(None), |value| {
        value
            .as_object()
            .cloned()
            .map(Some)
            .ok_or(GeminiToolsError::ToolValue { tool: name })
    })
}
fn declaration(
    function: &Object,
    settings: &Settings,
) -> Result<FunctionDeclaration, GeminiToolsError> {
    let Some(Json::Text(name)) = function.get("name") else {
        return Err(GeminiToolsError::FunctionField { field: "name" });
    };
    let description = match function.get("description") {
        Some(Json::Text(text)) => Some(text.clone()),
        Some(Json::Null) | None => None,
        Some(_) => {
            return Err(GeminiToolsError::FunctionField {
                field: "description",
            });
        }
    };
    let parameters = match function.get("parameters") {
        Some(Json::Object(parameters)) => {
            Some(Json::Object(shape_tool_parameters(parameters, settings)?))
        }
        Some(Json::Text(text)) if text.is_empty() => Some(Json::Object(Object::from([(
            "type".to_owned(),
            Json::Text("object".to_owned()),
        )]))),
        Some(Json::Null) | None => None,
        Some(_) => {
            return Err(GeminiToolsError::FunctionField {
                field: "parameters",
            });
        }
    };
    Ok(FunctionDeclaration {
        name: name.clone(),
        description,
        parameters,
    })
}
fn shape_tool_parameters(
    parameters: &Object,
    settings: &Settings,
) -> Result<Object, GeminiToolsError> {
    shape_parameters(
        parameters.clone(),
        SchemaFormat::ResponseSchema,
        false,
        settings,
    )
    .map_err(|source| GeminiToolsError::Shape { source })
}
