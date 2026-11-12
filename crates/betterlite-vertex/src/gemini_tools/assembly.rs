use std::mem;

use betterlite_config::sections::vertex_schema_shaping::Settings;
use betterlite_prompts::arguments::json::{Json, Object};

use super::cleanup;
use super::error::GeminiToolsError;
use super::records::{FunctionDeclaration, RetrievalConfig, Tool, ToolConfig};
use super::specials;
use super::tool_lookup::tool_value;
use crate::vertex_schema::build::{SchemaFormat, shape_parameters};

#[derive(Debug)]
pub struct MappedTools {
    pub tools: Vec<Tool>,
    pub tool_config: Option<ToolConfig>,
    pub search_conflict_resolved: bool,
    pub invalid_tools: usize,
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
        collected.apply(map_tool(&cleaned, settings)?);
    }
    let search_conflict_resolved = collected.resolve_search_conflict(include_server_side);
    let mut config = tool_config;
    if let Some(retrieval) = collected.retrieval.take() {
        let merged = config.get_or_insert_with(ToolConfig::default);
        merged.retrieval_config = Some(retrieval);
    }
    let invalid_tools = collected.invalid;
    Ok(MappedTools {
        tools: collected.assemble(),
        tool_config: config,
        search_conflict_resolved,
        invalid_tools,
    })
}

#[derive(Default)]
struct Collected {
    declarations: Vec<FunctionDeclaration>,
    google_search: Option<Object>,
    google_search_retrieval: Option<Object>,
    enterprise_web_search: Option<Object>,
    code_execution: Option<Object>,
    url_context: Option<Object>,
    google_maps: Option<Object>,
    computer_use: Option<Object>,
    retrieval: Option<RetrievalConfig>,
    invalid: usize,
}

enum Mapped {
    Declaration(FunctionDeclaration),
    GoogleSearch(Object),
    GoogleSearchRetrieval(Object),
    EnterpriseWebSearch(Object),
    CodeExecution(Object),
    UrlContext(Object),
    Maps(Object, Option<RetrievalConfig>),
    ComputerUse(Object),
    Nothing,
    Invalid,
}

impl Collected {
    fn apply(&mut self, mapped: Mapped) {
        match mapped {
            Mapped::Declaration(declaration) => self.declarations.push(declaration),
            Mapped::GoogleSearch(options) => self.google_search = Some(options),
            Mapped::GoogleSearchRetrieval(options) => self.google_search_retrieval = Some(options),
            Mapped::EnterpriseWebSearch(options) => self.enterprise_web_search = Some(options),
            Mapped::CodeExecution(options) => self.code_execution = Some(options),
            Mapped::UrlContext(options) => self.url_context = Some(options),
            Mapped::Maps(options, retrieval) => {
                self.google_maps = Some(options);
                self.retrieval = retrieval;
            }
            Mapped::ComputerUse(options) => self.computer_use = Some(options),
            Mapped::Nothing => {}
            Mapped::Invalid => self.invalid = self.invalid.saturating_add(1),
        }
    }

    fn resolve_search_conflict(&mut self, include_server_side: bool) -> bool {
        let has_search = self.google_search.is_some()
            || self.google_search_retrieval.is_some()
            || self.enterprise_web_search.is_some()
            || self.url_context.is_some();
        if self.declarations.is_empty() || !has_search || include_server_side {
            return false;
        }
        tracing::warn!(
            "function declarations cannot be mixed with search tools; dropping googleSearch, googleSearchRetrieval, enterpriseWebSearch and url_context"
        );
        self.google_search = None;
        self.google_search_retrieval = None;
        self.enterprise_web_search = None;
        self.url_context = None;
        true
    }

    fn assemble(mut self) -> Vec<Tool> {
        let mut tools = Vec::new();
        if !self.declarations.is_empty() {
            tools.push(Tool::FunctionDeclarations {
                function_declarations: mem::take(&mut self.declarations),
            });
        }
        tools.extend(self.special_tools());
        tools
    }

    fn special_tools(&mut self) -> Vec<Tool> {
        let mut tools = Vec::new();
        if let Some(options) = self.google_search.take() {
            tools.push(Tool::GoogleSearch {
                google_search: options,
            });
        }
        if let Some(options) = self.google_search_retrieval.take() {
            tools.push(Tool::GoogleSearchRetrieval {
                google_search_retrieval: options,
            });
        }
        if let Some(options) = self.enterprise_web_search.take() {
            tools.push(Tool::EnterpriseWebSearch {
                enterprise_web_search: options,
            });
        }
        if let Some(options) = self.code_execution.take() {
            tools.push(Tool::CodeExecution {
                code_execution: options,
            });
        }
        if let Some(options) = self.url_context.take() {
            tools.push(Tool::UrlContext {
                url_context: options,
            });
        }
        if let Some(options) = self.google_maps.take() {
            tools.push(Tool::GoogleMaps {
                google_maps: options,
            });
        }
        if let Some(options) = self.computer_use.take() {
            tools.push(Tool::ComputerUse {
                computer_use: options,
            });
        }
        tools
    }
}

fn map_tool(tool: &Object, settings: &Settings) -> Result<Mapped, GeminiToolsError> {
    let function_payload = function_object(tool)?;
    let transformed = specials::transform_type(tool);
    if let Some(name) = single_key(&transformed)
        && let Some(mapped) = map_special(name, &transformed)
    {
        return mapped;
    }
    function_payload.map_or_else(
        || Ok(Mapped::Invalid),
        |payload| declaration(&payload, settings).map(Mapped::Declaration),
    )
}

fn single_key(tool: &Object) -> Option<&str> {
    let mut keys = tool.keys();
    let name = keys.next()?;
    if keys.next().is_some() {
        return None;
    }
    Some(name.as_str())
}

fn function_object(tool: &Object) -> Result<Option<Object>, GeminiToolsError> {
    match tool.get("function") {
        Some(Json::Object(function)) => Ok(Some(function.clone())),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(GeminiToolsError::Function)
        }
        None => {
            if tool.contains_key("name") {
                Ok(Some(tool.clone()))
            } else {
                Ok(None)
            }
        }
    }
}

fn map_special(name: &str, tool: &Object) -> Option<Result<Mapped, GeminiToolsError>> {
    match name {
        "codeExecution" | "code_execution" => {
            Some(object_tool(tool, "codeExecution", Mapped::CodeExecution))
        }
        "googleSearch" | "google_search" => {
            Some(object_tool(tool, "googleSearch", Mapped::GoogleSearch))
        }
        "googleSearchRetrieval" | "google_search_retrieval" => Some(object_tool(
            tool,
            "googleSearchRetrieval",
            Mapped::GoogleSearchRetrieval,
        )),
        "enterpriseWebSearch" | "enterprise_web_search" => Some(object_tool(
            tool,
            "enterpriseWebSearch",
            Mapped::EnterpriseWebSearch,
        )),
        "urlContext" | "url_context" => Some(object_tool(tool, "urlContext", Mapped::UrlContext)),
        "googleMaps" | "google_maps" => Some(maps_tool(tool)),
        "computerUse" | "computer_use" => Some(computer_use_tool(tool)),
        _ => None,
    }
}

fn object_tool(
    tool: &Object,
    name: &'static str,
    wrap: fn(Object) -> Mapped,
) -> Result<Mapped, GeminiToolsError> {
    match tool_value(tool, name) {
        None => Ok(Mapped::Nothing),
        Some(Json::Object(options)) => Ok(wrap(options.clone())),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(GeminiToolsError::ToolValue { tool: name })
        }
    }
}

fn maps_tool(tool: &Object) -> Result<Mapped, GeminiToolsError> {
    match tool_value(tool, "googleMaps") {
        None => Ok(Mapped::Nothing),
        Some(Json::Object(config)) => {
            let (cleaned, retrieval) = specials::extract_maps(config);
            Ok(Mapped::Maps(cleaned, retrieval))
        }
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(GeminiToolsError::ToolValue { tool: "googleMaps" })
        }
    }
}

fn computer_use_tool(tool: &Object) -> Result<Mapped, GeminiToolsError> {
    match tool_value(tool, "computerUse") {
        None => Ok(Mapped::ComputerUse(Object::new())),
        Some(Json::Object(config)) => Ok(Mapped::ComputerUse(specials::transform_computer_use(
            config,
        ))),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            Err(GeminiToolsError::ToolValue {
                tool: "computerUse",
            })
        }
    }
}

fn declaration(
    function: &Object,
    settings: &Settings,
) -> Result<FunctionDeclaration, GeminiToolsError> {
    let Some(Json::Text(name)) = function.get("name") else {
        return Err(GeminiToolsError::FunctionName);
    };
    let description = match function.get("description") {
        None | Some(Json::Null) => None,
        Some(Json::Text(text)) => Some(text.clone()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            return Err(GeminiToolsError::Description);
        }
    };
    let parameters = match function.get("parameters") {
        None | Some(Json::Null) => None,
        Some(Json::Object(parameters)) => {
            let shaped = shape_parameters(
                parameters.clone(),
                SchemaFormat::ResponseSchema,
                false,
                settings,
            )
            .map_err(|source| GeminiToolsError::Shape { source })?;
            Some(Json::Object(shaped))
        }
        Some(Json::Text(text)) if text.is_empty() => {
            let mut empty = Object::new();
            empty.insert("type".to_owned(), Json::Text("object".to_owned()));
            Some(Json::Object(empty))
        }
        Some(Json::Text(_) | Json::Bool(_) | Json::Number(_) | Json::List(_)) => {
            return Err(GeminiToolsError::Parameters);
        }
    };
    Ok(FunctionDeclaration {
        name: name.clone(),
        description,
        parameters,
    })
}
