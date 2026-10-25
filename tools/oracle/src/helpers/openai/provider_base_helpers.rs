use std::collections::BTreeMap;

use betterlite_config::loader::{ConfigError, ConfigLoader};
use betterlite_config::types::LiteLlmSettings;
use betterlite_openai::base_helpers::{
    DUMMY_TOOLS, HiddenCarrier, HistoryMessage, ImageEditBase, ResponseKind, ResponsesBase,
    add_thinking_max_tokens, append_tools, assistant_has_thinking, has_tool_calls, hidden_params,
    last_tool_call_unthought, provider_config, response_format_refs, thinking_request,
};
use betterlite_openai::error::{OpenAiError, ProviderError, ProviderErrorParts};
use serde_json::{Map, Value};

use crate::helpers::HelperError;

mod tests;

type Object = Map<String, Value>;
type Environment = BTreeMap<String, String>;
type Outcome = Result<Value, Box<HelperError>>;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum BaseCall {
    BaseError(ProviderErrorParts),
    ClassConfig(ClassMembers),
    Thinking(Object),
    ThinkingMaxTokens(TokensCall),
    AppendTools(ToolsCall),
    ResponseFormat(FormatCall),
    HiddenParams(HiddenCall),
    History(Vec<HistoryMessage>),
    DummyTool(()),
    ModifyParams(ModifyCall),
    ResponsesInit(()),
    ImageEditInit(()),
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClassMembers {
    values: Object,
    // Only Python reads these: a Rust record has data fields and no class attributes
    methods: BTreeMap<String, MethodKind>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MethodKind {
    Function,
    Builtin,
    Classmethod,
    Staticmethod,
    Property,
    Callable,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TokensCall {
    requested: Object,
    options: Object,
    environment: Environment,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolsCall {
    options: Object,
    tools: Vec<Value>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FormatCall {
    format: Option<Object>,
    ref_template: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HiddenCall {
    kind: ResponseKind,
    hidden: Hidden,
    create: bool,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Hidden {
    Missing,
    Opaque,
    Mapping(Object),
    Model(Object),
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModifyCall {
    environment: Environment,
    configured: Option<bool>,
}

pub(crate) fn invoke(call: &BaseCall) -> Outcome {
    let encode = |source| OpenAiError::Encode { source };
    let value = match call.clone() {
        BaseCall::BaseError(parts) => serde_json::to_value(ProviderError::from(parts)),
        BaseCall::ClassConfig(members) => Ok(Value::Object(provider_config(&members.values)?)),
        BaseCall::Thinking(params) => serde_json::to_value(thinking_request(&params)),
        BaseCall::ThinkingMaxTokens(input) => return input.run(),
        BaseCall::AppendTools(mut input) => {
            append_tools(&mut input.options, input.tools)?;
            Ok(Value::Object(input.options))
        }
        BaseCall::ResponseFormat(input) => {
            let template = input.ref_template.as_deref();
            serde_json::to_value(response_format_refs(input.format.as_ref(), template)?)
        }
        BaseCall::HiddenParams(input) => input.run(),
        BaseCall::History(messages) => serde_json::to_value([
            has_tool_calls(&messages),
            assistant_has_thinking(&messages),
            last_tool_call_unthought(&messages),
        ]),
        BaseCall::DummyTool(()) => serde_json::to_value(DUMMY_TOOLS),
        BaseCall::ModifyParams(input) => return input.run(),
        BaseCall::ResponsesInit(()) => serde_json::to_value(Vec::from_iter(
            provider_config(&ResponsesBase)?
                .into_iter()
                .map(|(name, _)| name),
        )),
        BaseCall::ImageEditInit(()) => serde_json::to_value(Vec::from_iter(
            provider_config(&ImageEditBase)?
                .into_iter()
                .map(|(name, _)| name),
        )),
    };
    Ok(value.map_err(encode)?)
}

fn load_settings(lookup: Environment, source: &str) -> Result<LiteLlmSettings, ConfigError> {
    let loaded = ConfigLoader { lookup }.load(source)?;
    Ok(loaded.litellm_settings)
}

impl TokensCall {
    fn run(mut self) -> Outcome {
        let default_max_tokens = load_settings(self.environment, "")?.default_max_tokens;
        add_thinking_max_tokens(&self.requested, &mut self.options, default_max_tokens)?;
        Ok(Value::Object(self.options))
    }
}

impl HiddenCall {
    fn run(self) -> Result<Value, serde_json::Error> {
        let attached = match self.hidden {
            Hidden::Mapping(fields) | Hidden::Model(fields) => Some(fields),
            Hidden::Missing | Hidden::Opaque => None,
        };
        let mut carrier = HiddenCarrier {
            kind: self.kind,
            hidden_params: attached,
        };
        let projected = hidden_params(&mut carrier, self.create);
        serde_json::to_value((projected, carrier.hidden_params))
    }
}

impl ModifyCall {
    fn run(self) -> Outcome {
        let source = self.configured.map_or_else(String::new, |value| {
            format!("litellm_settings:\n  modify_params: {value}\n")
        });
        Ok(Value::Bool(
            load_settings(self.environment, &source)?.modify_params,
        ))
    }
}

impl From<OpenAiError> for Box<HelperError> {
    fn from(error: OpenAiError) -> Self {
        let class = match &error {
            OpenAiError::MissingKey { .. } | OpenAiError::TemplateField { .. } => "KeyError",
            OpenAiError::NotSubscriptable { .. }
            | OpenAiError::ToolsNotList
            | OpenAiError::BudgetNotInteger => "TypeError",
            OpenAiError::RefNotText => "AttributeError",
            OpenAiError::TemplatePosition => "IndexError",
            OpenAiError::TemplateBrace => "ValueError",
            OpenAiError::Provider(_)
            | OpenAiError::BudgetOverflow
            | OpenAiError::ConfigNotObject
            | OpenAiError::Encode { .. } => "OpenAiError",
        };
        Self::new(HelperError {
            class: class.to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    }
}

impl From<ConfigError> for Box<HelperError> {
    fn from(error: ConfigError) -> Self {
        Self::new(HelperError {
            class: "ConfigError".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: error.to_string(),
        })
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = BaseCall;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        invoke(input)
    }
}
