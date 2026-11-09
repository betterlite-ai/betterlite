use betterlite_context::bedrock_model::get_bedrock_base_model;
use betterlite_context::chat_config_selection::foundry::{self, AzureAiRoute};
use betterlite_context::chat_config_selection::image_variation::select_image_variation_config;
use betterlite_context::chat_config_selection::selection::select_chat_config;
use betterlite_context::chat_config_selection::table::ProviderConfigTable;
use betterlite_context::chat_config_selection::tags::{
    ChatConfigTag, ConfigConstruction, ConfigSelection,
};
use serde_json::{Value, json};

use super::fixtures::Fixtures;
use super::{Call, invalid};
use crate::helpers::HelperError;

pub(super) fn report_each<Report>(
    calls: &[Call],
    mut report: Report,
) -> Result<Value, Box<HelperError>>
where
    Report: FnMut(&mut ProviderConfigTable, &Call) -> Result<Value, Box<HelperError>>,
{
    let mut table = ProviderConfigTable::default();
    let mut reports = Vec::with_capacity(calls.len());
    for call in calls {
        reports.push(report(&mut table, call)?);
    }
    Ok(Value::Array(reports))
}

fn arguments_of<Tag>(selection: &ConfigSelection<Tag>) -> (Vec<String>, bool) {
    match &selection.construction {
        ConfigConstruction::Shared => (Vec::new(), false),
        ConfigConstruction::New(arguments) => (arguments.clone(), true),
    }
}

fn selected_report(class: &'static str, construction: &ConfigConstruction) -> Value {
    match construction {
        ConfigConstruction::Shared => {
            json!({"class": class, "arguments": [], "constructed": false})
        }
        ConfigConstruction::New(arguments) => {
            json!({"class": class, "arguments": arguments, "constructed": true})
        }
    }
}

fn select(
    fixtures: &Fixtures<'_>,
    call: &Call,
    table: &mut ProviderConfigTable,
) -> Result<Option<ConfigSelection<ChatConfigTag>>, Box<HelperError>> {
    let base_model = get_bedrock_base_model(&call.model);
    let request = fixtures.request(call, &base_model)?;
    select_chat_config(&request, table).map_err(|error| invalid(&error.to_string()))
}

pub(super) fn chat_call(
    fixtures: &Fixtures<'_>,
    call: &Call,
    table: &mut ProviderConfigTable,
) -> Result<Value, Box<HelperError>> {
    match select(fixtures, call, table)? {
        Some(selection) => Ok(selected_report(
            selection.tag.class_name(),
            &selection.construction,
        )),
        None => Ok(Value::Null),
    }
}

const fn route_name(route: AzureAiRoute) -> &'static str {
    match route {
        AzureAiRoute::Agents => "agents",
        AzureAiRoute::ModelRouter => "model_router",
        AzureAiRoute::Default => "default",
    }
}

pub(super) fn foundry_call(
    fixtures: &Fixtures<'_>,
    call: &Call,
    table: &mut ProviderConfigTable,
) -> Result<Value, Box<HelperError>> {
    let route = route_name(foundry::get_azure_ai_route(&call.model));
    let selection = select(fixtures, call, table)?
        .ok_or_else(|| invalid("the Azure AI selection returned nothing"))?;
    let class = selection.tag.class_name();
    let (arguments, _) = arguments_of(&selection);
    Ok(json!({"route": route, "class": class, "arguments": arguments, "constructed": true}))
}

fn json_call(fixtures: &Fixtures<'_>, call: &Call, table: &mut ProviderConfigTable) -> Value {
    let provider = call.provider.to_string();
    match select(fixtures, call, table) {
        Ok(Some(selection)) => {
            let class = selection.tag.class_name();
            let (arguments, constructed) = arguments_of(&selection);
            json!({"outcome": "selected", "class": class, "arguments": arguments, "constructed": constructed})
        }
        Ok(None) => json!({"outcome": "absent"}),
        Err(_) => json!({"outcome": "error", "provider": provider}),
    }
}

pub(super) fn json_reports(fixtures: &Fixtures<'_>, calls: &[Call]) -> Value {
    let mut table = ProviderConfigTable::default();
    let mut reports = Vec::with_capacity(calls.len());
    for call in calls {
        reports.push(json_call(fixtures, call, &mut table));
    }
    Value::Array(reports)
}

pub(super) fn image_report(call: &Call) -> Value {
    match select_image_variation_config(&call.model, call.provider) {
        Some(selection) => selected_report(selection.tag.class_name(), &selection.construction),
        None => Value::Null,
    }
}
