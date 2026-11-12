use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_models::request_pricing::custom_fields::CUSTOM_PRICING_FIELDS;
use betterlite_models::request_pricing::entry::{build_custom_pricing_entry, router_deployment_id};
use betterlite_models::request_pricing::fields::{
    COST_MAP_LOOKUP_KEY, DEPLOYMENT_SCOPED_FIELDS, PRICING_FIELD_EXEMPTIONS, PRICING_OVERRIDES_KEY,
    echoed_cost_map_fields, echoed_cost_map_pricing_fields, is_server_derived_pricing_key,
    pricing_override_fields, shared_backend_model_info, strip_custom_pricing_fields,
    without_server_derived_pricing,
};
use betterlite_models::request_pricing::shared_fields::SHARED_MODEL_INFO_FIELDS;
use betterlite_sdk::call_setup::RequestMap;
use betterlite_sdk::completion_options::control::resolve_control_options;
use betterlite_sdk::completion_options::error::CompletionOptionsError;
use betterlite_sdk::completion_options::tool_keys::{
    drop_anthropic_only_tool_keys, is_claude_tool_target,
};
use project::project;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult, OrHelperError as _};

mod project;

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum Input {
    Options(OptionsCall),
    Objects(ObjectsCall),
    Lists(ListsCall),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum OptionsCall {
    Control(ControlArgs),
    ToolKeys(ToolsArgs),
    PricingEntry(EntryArgs),
    DeploymentId(IdArgs),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum ObjectsCall {
    Strip(ModelInfoArgs),
    SharedInfo(ModelInfoArgs),
    WithoutServerDerived(ModelInfoArgs),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum ListsCall {
    ClaudeTarget(ClaudeArgs),
    CustomFields,
    SharedFields,
    Constants,
    ServerDerivedKey(KeyArgs),
    EchoedPricing(ModelInfoArgs),
    EchoedFields(EchoedArgs),
    OverrideFields(SourcesArgs),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ControlArgs {
    kwargs: RequestMap,
    model: String,
    global_drop_params: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClaudeArgs {
    custom_llm_provider: Option<String>,
    model: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolsArgs {
    tools: Option<Vec<Json>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EntryArgs {
    custom_llm_provider: String,
    kwargs: JsonObject,
    model_info: Option<JsonObject>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IdArgs {
    kwargs: JsonObject,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct KeyArgs {
    key: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelInfoArgs {
    model_info: JsonObject,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EchoedArgs {
    model_info: JsonObject,
    entries: Vec<JsonObject>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SourcesArgs {
    sources: Vec<JsonObject>,
}

fn sorted_names(names: &[&str]) -> Value {
    let mut sorted = names.to_vec();
    sorted.sort_unstable();
    json!(sorted)
}

impl ControlArgs {
    fn run(&self) -> HelperResult {
        let resolved = resolve_control_options(&self.kwargs, &self.model, self.global_drop_params);
        match resolved {
            Ok(control) => Ok(json!({"stream_chunk_size": control.stream_chunk_size})),
            Err(CompletionOptionsError::InvalidControlOption(details)) => {
                Err(Box::new(HelperError {
                    class: "BadRequestError".to_owned(),
                    status: Some(details.status.0),
                    error_type: Some(None),
                    param: Some(None),
                    code: Some(None),
                    message: details.message,
                }))
            }
        }
    }
}

impl ToolsArgs {
    fn run(&self) -> HelperResult {
        let kept = drop_anthropic_only_tool_keys(self.tools.as_deref());
        let shown = kept.map_or(Ok(Value::Null), |list| project(&Json::List(list)))?;
        Ok(json!({"tools": shown}))
    }
}

impl EntryArgs {
    fn run(&self) -> HelperResult {
        let model_info = self.model_info.as_ref();
        let entry = build_custom_pricing_entry(&self.custom_llm_provider, &self.kwargs, model_info);
        project(&Json::Object(entry))
    }
}

fn options(call: &OptionsCall) -> HelperResult {
    match call {
        OptionsCall::Control(args) => args.run(),
        OptionsCall::ToolKeys(args) => args.run(),
        OptionsCall::PricingEntry(args) => args.run(),
        OptionsCall::DeploymentId(args) => {
            Ok(json!(router_deployment_id(&args.kwargs).or_value_error()?))
        }
    }
}

fn constants() -> Value {
    json!({
        "deployment_scoped": sorted_names(DEPLOYMENT_SCOPED_FIELDS),
        "exemptions": sorted_names(PRICING_FIELD_EXEMPTIONS),
        "overrides_key": PRICING_OVERRIDES_KEY,
        "lookup_key": COST_MAP_LOOKUP_KEY,
    })
}

fn objects(call: &ObjectsCall) -> HelperResult {
    let (args, convert): (&ModelInfoArgs, fn(&JsonObject) -> JsonObject) = match call {
        ObjectsCall::Strip(args) => (args, strip_custom_pricing_fields),
        ObjectsCall::SharedInfo(args) => (args, shared_backend_model_info),
        ObjectsCall::WithoutServerDerived(args) => (args, without_server_derived_pricing),
    };
    project(&Json::Object(convert(&args.model_info)))
}

fn lists(call: &ListsCall) -> Value {
    match call {
        ListsCall::ClaudeTarget(args) => json!(is_claude_tool_target(
            args.custom_llm_provider.as_deref(),
            &args.model
        )),
        ListsCall::CustomFields => sorted_names(CUSTOM_PRICING_FIELDS),
        ListsCall::SharedFields => sorted_names(SHARED_MODEL_INFO_FIELDS),
        ListsCall::Constants => constants(),
        ListsCall::ServerDerivedKey(args) => json!(is_server_derived_pricing_key(&args.key)),
        ListsCall::EchoedPricing(args) => json!(echoed_cost_map_pricing_fields(&args.model_info)),
        ListsCall::EchoedFields(args) => json!(echoed_cost_map_fields(
            &args.model_info,
            &args.entries.iter().collect::<Vec<_>>()
        )),
        ListsCall::OverrideFields(args) => json!(pricing_override_fields(
            &args.sources.iter().collect::<Vec<_>>()
        )),
    }
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Options(call) => options(call),
            Input::Objects(call) => objects(call),
            Input::Lists(call) => Ok(lists(call)),
        }
    }
}
