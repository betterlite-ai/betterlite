use crate::helpers::OrHelperError as _;
use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
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
    let openai = |error: OpenAiError| openai_helper_error(&error);
    let value = match call.clone() {
        BaseCall::BaseError(parts) => serde_json::to_value(ProviderError::from(parts)),
        BaseCall::ClassConfig(members) => Ok(Value::Object(
            provider_config(&members.values).map_err(openai)?,
        )),
        BaseCall::Thinking(params) => serde_json::to_value(thinking_request(&params)),
        BaseCall::ThinkingMaxTokens(input) => return input.run().map(Value::Object),
        BaseCall::AppendTools(mut input) => {
            append_tools(&mut input.options, input.tools).map_err(openai)?;
            Ok(Value::Object(input.options))
        }
        BaseCall::ResponseFormat(input) => {
            let template = input.ref_template.as_deref();
            serde_json::to_value(
                response_format_refs(input.format.as_ref(), template).map_err(openai)?,
            )
        }
        BaseCall::HiddenParams(input) => input.run(),
        BaseCall::History(messages) => serde_json::to_value([
            has_tool_calls(&messages),
            assistant_has_thinking(&messages),
            last_tool_call_unthought(&messages),
        ]),
        BaseCall::DummyTool(()) => serde_json::to_value(DUMMY_TOOLS),
        BaseCall::ModifyParams(input) => return input.run(),
        BaseCall::ResponsesInit(()) => serde_json::to_value(
            provider_config(&ResponsesBase)
                .map_err(openai)?
                .keys()
                .collect::<Vec<_>>(),
        ),
        BaseCall::ImageEditInit(()) => serde_json::to_value(
            provider_config(&ImageEditBase)
                .map_err(openai)?
                .keys()
                .collect::<Vec<_>>(),
        ),
    };
    value.map_err(|source| openai(OpenAiError::Encode { source }))
}

impl TokensCall {
    fn run(self) -> Result<Object, Box<HelperError>> {
        let default_max_tokens = load_settings(self.environment, "")?.default_max_tokens;
        let mut options = self.options;
        add_thinking_max_tokens(&self.requested, &mut options, default_max_tokens)
            .map_err(|error| openai_helper_error(&error))?;
        Ok(options)
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
        let source = match self.configured {
            None => "",
            Some(true) => "litellm_settings: {modify_params: true}",
            Some(false) => "litellm_settings: {modify_params: false}",
        };
        Ok(Value::Bool(
            load_settings(self.environment, source)?.modify_params,
        ))
    }
}

fn load_settings(
    environment: Environment,
    source: &str,
) -> Result<LiteLlmSettings, Box<HelperError>> {
    let loaded = ConfigLoader {
        lookup: environment,
    }
    .load(source);
    Ok(loaded.or_config_error()?.litellm_settings)
}

fn openai_helper_error(error: &OpenAiError) -> Box<HelperError> {
    let class = match error {
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
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = BaseCall;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        invoke(input)
    }
}
