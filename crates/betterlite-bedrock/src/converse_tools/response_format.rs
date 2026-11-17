use betterlite_anthropic::anthropic_capability_catalog::CatalogSources;
use betterlite_anthropic::anthropic_capability_catalog::forced_tool::forced_tool_use_unsupported;
use betterlite_logging::python_json::dumps;
use betterlite_models::decisions::Json;
use betterlite_models::model_capability_predicates::{
    CapabilityPredicates, CapabilityRequest, NamedCapability,
};
use indexmap::IndexMap;

use crate::converse_content::json_mode::RESPONSE_FORMAT_TOOL_NAME;
use crate::converse_options::config::CUSTOM_LLM_PROVIDER;
use crate::converse_tools::additional_properties::add_additional_properties;
use crate::converse_tools::error::ConverseToolsError;
use crate::converse_tools::json_mode::{append_tools, create_json_tool_call};
use crate::converse_tools::tool_choice::specific_tool_choice;

pub struct ResponseFormatRequest<'request, 'catalog> {
    pub model: &'request str,
    pub is_thinking_enabled: bool,
    pub stream: &'request Json,
    pub catalogs: &'request CatalogSources<'catalog>,
}

enum Format<'value> {
    Text,
    Json {
        schema: Option<&'value Json>,
        name: Option<&'value str>,
        description: Option<&'value str>,
    },
}

const NATIVE_STRUCTURED_OUTPUT: NamedCapability = NamedCapability::NativeStructuredOutput;

pub async fn translate_response_format<Capabilities>(
    value: &Json,
    request: &ResponseFormatRequest<'_, '_>,
    optional_params: &mut IndexMap<String, Json>,
    capabilities: &mut Capabilities,
) -> Result<(), ConverseToolsError>
where
    Capabilities: CapabilityPredicates,
{
    let Format::Json {
        schema: schema_field,
        name,
        description,
    } = read_format(value)?
    else {
        return Ok(());
    };
    if let Some(schema) = schema_field {
        if capability_supported(request, NATIVE_STRUCTURED_OUTPUT, capabilities).await {
            optional_params.insert(
                "outputConfig".to_owned(),
                create_output_config(Some(schema), name, description),
            );
        } else {
            apply_tool_fallback(schema, description, request, optional_params, capabilities)
                .await?;
        }
    }
    optional_params.insert("json_mode".to_owned(), Json::Bool(true));
    Ok(())
}

#[must_use]
pub fn create_output_config(
    json_schema: Option<&Json>,
    name: Option<&str>,
    description: Option<&str>,
) -> Json {
    let schema = json_schema.map_or_else(
        || "{}".to_owned(),
        |schema| dumps(&add_additional_properties(schema)),
    );
    let mut definition = IndexMap::new();
    definition.insert("schema".to_owned(), Json::Text(schema));
    if let Some(label) = name {
        definition.insert("name".to_owned(), Json::Text(label.to_owned()));
    }
    if let Some(note) = description {
        definition.insert("description".to_owned(), Json::Text(note.to_owned()));
    }
    let mut structure = IndexMap::new();
    structure.insert("jsonSchema".to_owned(), Json::Object(definition));
    let mut text_format = IndexMap::new();
    text_format.insert("type".to_owned(), Json::Text("json_schema".to_owned()));
    text_format.insert("structure".to_owned(), Json::Object(structure));
    let mut config = IndexMap::new();
    config.insert("textFormat".to_owned(), Json::Object(text_format));
    Json::Object(config)
}

fn read_format(value: &Json) -> Result<Format<'_>, ConverseToolsError> {
    let Json::Object(fields) = value else {
        return Err(ConverseToolsError::FormatNotAnObject);
    };
    let Some(format_type) = fields.get("type") else {
        return Err(ConverseToolsError::MissingFormatType);
    };
    if matches!(format_type, Json::Text(text) if text == "text") {
        return Ok(Format::Text);
    }
    let (schema, name, description) = read_schema(fields)?;
    Ok(Format::Json {
        schema,
        name,
        description,
    })
}

type SchemaFields<'value> = (
    Option<&'value Json>,
    Option<&'value str>,
    Option<&'value str>,
);

fn read_schema(fields: &IndexMap<String, Json>) -> Result<SchemaFields<'_>, ConverseToolsError> {
    if let Some(candidate) = fields.get("response_schema") {
        let schema = (!matches!(candidate, Json::Null)).then_some(candidate);
        return Ok((schema, None, None));
    }
    let Some(raw) = fields.get("json_schema") else {
        return Ok((None, None, None));
    };
    let Json::Object(envelope) = raw else {
        return Err(ConverseToolsError::JsonSchemaNotAnObject);
    };
    let Some(candidate) = envelope.get("schema") else {
        return Err(ConverseToolsError::MissingSchemaField);
    };
    let schema = (!matches!(candidate, Json::Null)).then_some(candidate);
    Ok((
        schema,
        text_field(envelope, "name")?,
        text_field(envelope, "description")?,
    ))
}

fn text_field<'envelope>(
    envelope: &'envelope IndexMap<String, Json>,
    field: &'static str,
) -> Result<Option<&'envelope str>, ConverseToolsError> {
    match envelope.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text)),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(ConverseToolsError::NotText { field })
        }
    }
}

async fn apply_tool_fallback<Capabilities>(
    schema: &Json,
    description: Option<&str>,
    request: &ResponseFormatRequest<'_, '_>,
    optional_params: &mut IndexMap<String, Json>,
    capabilities: &mut Capabilities,
) -> Result<(), ConverseToolsError>
where
    Capabilities: CapabilityPredicates,
{
    let tool = create_json_tool_call(Some(schema), description);
    append_tools(optional_params, vec![tool])?;
    if capability_supported(request, NamedCapability::ToolChoice, capabilities).await
        && !request.is_thinking_enabled
        && !forced_tool_use_unsupported(request.model, request.catalogs)
    {
        optional_params.insert(
            "tool_choice".to_owned(),
            Json::Object(specific_tool_choice(Json::Text(
                RESPONSE_FORMAT_TOOL_NAME.to_owned(),
            ))),
        );
    }
    if matches!(request.stream, Json::Bool(true)) {
        optional_params.insert("fake_stream".to_owned(), Json::Bool(true));
    }
    Ok(())
}

async fn capability_supported<Capabilities>(
    request: &ResponseFormatRequest<'_, '_>,
    capability: NamedCapability,
    capabilities: &mut Capabilities,
) -> bool
where
    Capabilities: CapabilityPredicates,
{
    capabilities
        .supports_factory(
            &CapabilityRequest {
                model: request.model,
                custom_llm_provider: Some(CUSTOM_LLM_PROVIDER),
            },
            capability.key(),
        )
        .await
}
