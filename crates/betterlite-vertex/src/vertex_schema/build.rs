use betterlite_config::sections::vertex_schema_shaping::Settings;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::definitions::expand::unpack_defs;

use super::anyof::convert_anyof_null_to_nullable;
use super::enums::{fix_enum_empty_strings, fix_enum_types};
use super::error::VertexSchemaError;
use super::filter::filter_schema_fields;
use super::items::process_items;
use super::object_type::add_object_type;
use super::ordering::set_schema_property_ordering;
use super::recursion::Recursion;
use super::type_arrays::convert_schema_types;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaFormat {
    ResponseSchema,
    JsonSchema,
}

pub fn shape_parameters(
    parameters: Object,
    format: SchemaFormat,
    add_property_ordering: bool,
    settings: &Settings,
) -> Result<Object, VertexSchemaError> {
    match format {
        // Gemini 2.0+ takes standard JSON Schema for responseJsonSchema, so refs and unknown keys reach Vertex as the caller wrote them
        SchemaFormat::JsonSchema => Ok(parameters),
        SchemaFormat::ResponseSchema => build_response_schema(
            parameters,
            add_property_ordering,
            settings.max_recurse_depth,
        ),
    }
}

// The order is LiteLLM's: the null pass rewrites anyOf before the type pass creates anyOf from a type array
fn build_response_schema(
    mut parameters: Object,
    add_property_ordering: bool,
    max_depth: usize,
) -> Result<Object, VertexSchemaError> {
    let definitions = match parameters.shift_remove("$defs") {
        None => Object::new(),
        Some(Json::Object(definitions)) => definitions,
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(VertexSchemaError::DefinitionsNotObject);
        }
    };
    let mut shaped = unpack_defs(parameters, &definitions, None)?;
    let recursion = Recursion { max_depth };
    convert_anyof_null_to_nullable(&mut shaped, recursion, 0)?;
    convert_schema_types(&mut shaped, recursion, 0)?;
    fix_enum_empty_strings(&mut shaped, recursion, 0)?;
    fix_enum_types(&mut shaped, recursion, 0)?;
    process_items(&mut shaped, recursion, 0)?;
    add_object_type(&mut shaped);
    let mut filtered = filter_schema_fields(&shaped);
    if add_property_ordering {
        set_schema_property_ordering(&mut filtered, recursion, 0)?;
    }
    Ok(filtered)
}
