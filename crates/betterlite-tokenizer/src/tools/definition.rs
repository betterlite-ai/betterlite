use serde_json::value::RawValue;

use crate::tools::error::ToolFormatError;
use crate::tools::schema::{self, Fields};

struct Function {
    name: Option<String>,
    description: Option<String>,
    parameters: Option<Fields>,
}

pub(super) fn format_tool(tool: &RawValue, lines: &mut Vec<String>) -> Result<(), ToolFormatError> {
    let Ok(fields) = serde_json::from_str::<Fields>(tool.get()) else {
        return Ok(());
    };
    for function in functions_for_tool(&fields)? {
        lines.extend(format_function(&function)?);
    }
    Ok(())
}

fn functions_for_tool(fields: &Fields) -> Result<Vec<Function>, ToolFormatError> {
    if let Some(raw) = schema::present(fields, "function") {
        let function = serde_json::from_str::<Fields>(raw.get())
            .ok()
            .ok_or(ToolFormatError::NotAnObject { field: "function" })?;
        return Ok(vec![function_from(&function)?]);
    }
    if let Some(functions) = declaration_functions(fields)? {
        return Ok(functions);
    }
    let parameters = inline_parameters(fields)?;
    Ok(vec![Function {
        name: schema::text(fields, "name")?,
        description: schema::text(fields, "description")?,
        parameters,
    }])
}

fn inline_parameters(fields: &Fields) -> Result<Option<Fields>, ToolFormatError> {
    let (key, raw) = schema::present(fields, "input_schema").map_or_else(
        || ("parameters", schema::present(fields, "parameters")),
        |raw| ("input_schema", Some(raw)),
    );
    schema::object_from(raw, key)
}

fn declaration_functions(fields: &Fields) -> Result<Option<Vec<Function>>, ToolFormatError> {
    let declarations = schema::present(fields, "function_declarations")
        .or_else(|| schema::present(fields, "functionDeclarations"));
    let Some(raw) = declarations else {
        return Ok(None);
    };
    let entries: Vec<Box<RawValue>> =
        serde_json::from_str(raw.get()).map_err(|_error| ToolFormatError::NotAnArray {
            field: "functionDeclarations",
        })?;
    let mut functions = Vec::new();
    for entry in &entries {
        let declaration = serde_json::from_str::<Fields>(entry.get()).ok().ok_or(
            ToolFormatError::NotAnObject {
                field: "functionDeclarations",
            },
        )?;
        functions.push(function_from(&declaration)?);
    }
    Ok(Some(functions))
}

fn function_from(fields: &Fields) -> Result<Function, ToolFormatError> {
    Ok(Function {
        name: schema::text(fields, "name")?,
        description: schema::text(fields, "description")?,
        parameters: schema::object_from(schema::present(fields, "parameters"), "parameters")?,
    })
}

fn format_function(function: &Function) -> Result<Vec<String>, ToolFormatError> {
    // a function with no name has no type declaration to emit, as in LiteLLM
    let Some(name) = function.name.as_deref().filter(|name| !name.is_empty()) else {
        return Ok(Vec::new());
    };
    let mut lines = Vec::new();
    if let Some(description) = &function.description {
        lines.push(format!("// {description}"));
    }
    let Some(parameters) = &function.parameters else {
        return Ok(single_parameter_list(lines, name));
    };
    let properties = schema::object_from(schema::present(parameters, "properties"), "properties")?;
    if properties.is_none_or(|found| found.is_empty()) {
        return Ok(single_parameter_list(lines, name));
    }
    lines.push(format!("type {name} = (_: {{"));
    lines.push(schema::object_parameters(parameters, 0)?);
    lines.push("}) => any;".to_owned());
    lines.push(String::new());
    Ok(lines)
}

fn single_parameter_list(mut lines: Vec<String>, name: &str) -> Vec<String> {
    lines.push(format!("type {name} = () => any;"));
    lines.push(String::new());
    lines
}
