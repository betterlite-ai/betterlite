use std::collections::BTreeMap;

use thiserror::Error;

mod tests;

pub mod records;

use records::{
    AssemblyInput, AssemblyOutput, Choice, Chunk, CustomOutput, FunctionOutput, JsonNumber,
    JsonValue, ToolCall, ToolOutput,
};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AssemblyError {
    #[error("the function call input is empty")]
    EmptyFunctionInput,
    #[error("the first choice is missing")]
    MissingFirstChoice,
    #[error("the first choice has no function call")]
    MissingFirstFunction,
    #[error("the first function call has no name")]
    MissingFirstFunctionName,
    #[error("function call arguments cannot be joined")]
    InvalidArguments,
}

pub fn assemble(input: &AssemblyInput) -> Result<AssemblyOutput, AssemblyError> {
    match input {
        AssemblyInput::Tool { tool } => assemble_tools(&tool.chunks),
        AssemblyInput::Function { function } => assemble_function(&function.chunks),
    }
}

fn assemble_tools(chunks: &[Chunk]) -> Result<AssemblyOutput, AssemblyError> {
    let (calls, fragments) = collect_tool_states(chunks)?;
    let output = calls
        .into_iter()
        .filter_map(|(key, state)| tool_output(key, state, &fragments))
        .collect();
    Ok(AssemblyOutput::Tools(output))
}

fn collect_tool_states(chunks: &[Chunk]) -> Result<ToolStates, AssemblyError> {
    let mut calls: BTreeMap<(u32, u32), ToolState> = BTreeMap::new();
    let mut fragments: BTreeMap<((u32, u32), FragmentField), String> = BTreeMap::new();
    for chunk in chunks {
        for choice in &chunk.choices {
            for tool in choice
                .delta
                .tool_calls
                .as_deref()
                .unwrap_or_default()
                .iter()
                .flatten()
                .filter(|tool| tool.function.is_some() || tool.custom.is_some())
            {
                let key = (choice.index.unwrap_or(0), tool.index.unwrap_or(0));
                calls.entry(key).or_default().apply(tool);
                append_fragments(&mut fragments, key, tool)?;
            }
        }
    }
    Ok((calls, fragments))
}

fn tool_output(
    key: (u32, u32),
    state: ToolState,
    fragments: &BTreeMap<((u32, u32), FragmentField), String>,
) -> Option<ToolOutput> {
    let id = state.id?;
    if let Some(name) = state.custom_name {
        return Some(ToolOutput {
            id,
            kind: "custom".to_owned(),
            function: None,
            custom: Some(CustomOutput {
                name,
                input: fragments
                    .get(&(key, FragmentField::CustomInput))
                    .cloned()
                    .unwrap_or_default(),
            }),
            provider_specific_fields: None,
        });
    }
    let name = state.name?;
    let arguments = fragments
        .get(&(key, FragmentField::Arguments))
        .cloned()
        .unwrap_or_else(|| "{}".to_owned());
    Some(ToolOutput {
        id,
        kind: state.kind.unwrap_or_else(|| "function".to_owned()),
        function: Some(FunctionOutput { name, arguments }),
        custom: None,
        provider_specific_fields: state.provider_fields.filter(|fields| !fields.is_empty()),
    })
}

fn append_fragments(
    fragments: &mut BTreeMap<((u32, u32), FragmentField), String>,
    key: (u32, u32),
    tool: &ToolCall,
) -> Result<(), AssemblyError> {
    append_fragment(
        fragments,
        (key, FragmentField::Arguments),
        tool.function
            .as_ref()
            .and_then(|function| function.arguments.as_ref()),
    )?;
    append_fragment(
        fragments,
        (key, FragmentField::CustomInput),
        tool.custom
            .as_ref()
            .and_then(|custom| custom.input.as_ref()),
    )?;
    Ok(())
}

fn append_fragment(
    fragments: &mut BTreeMap<((u32, u32), FragmentField), String>,
    key: ((u32, u32), FragmentField),
    value: Option<&JsonValue>,
) -> Result<(), AssemblyError> {
    if let Some(fragment) = value.filter(|candidate| candidate.is_truthy()) {
        append_string(fragments, key, fragment)?;
    }
    Ok(())
}

fn append_string(
    fragments: &mut BTreeMap<((u32, u32), FragmentField), String>,
    key: ((u32, u32), FragmentField),
    value: &JsonValue,
) -> Result<(), AssemblyError> {
    if !value.is_truthy() {
        return Ok(());
    }
    let JsonValue::String(text) = value else {
        return Err(AssemblyError::InvalidArguments);
    };
    fragments.entry(key).or_default().push_str(text);
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum FragmentField {
    Arguments,
    CustomInput,
}

type ToolStates = (
    BTreeMap<(u32, u32), ToolState>,
    BTreeMap<((u32, u32), FragmentField), String>,
);

#[derive(Debug, Default)]
struct ToolState {
    id: Option<String>,
    kind: Option<String>,
    name: Option<String>,
    custom_name: Option<String>,
    provider_fields: Option<BTreeMap<String, JsonValue>>,
}

impl ToolState {
    fn apply(&mut self, tool: &ToolCall) {
        if let Some(value) = tool.id.as_ref().filter(|value| !value.is_empty()) {
            self.id = Some(value.clone());
        }
        if let Some(value) = tool.kind.as_ref().filter(|value| !value.is_empty()) {
            self.kind = Some(value.clone());
        }
        if let Some(value) = tool
            .function
            .as_ref()
            .and_then(|function| function.name.as_ref())
            .filter(|value| !value.is_empty())
        {
            self.name = Some(value.clone());
        }
        if let Some(value) = tool
            .custom
            .as_ref()
            .and_then(|custom| custom.name.as_ref())
            .filter(|value| !value.is_empty())
        {
            self.custom_name = Some(value.clone());
        }
        let tool_provider_fields = tool
            .provider_specific_fields
            .as_ref()
            .filter(|value| value.is_truthy());
        let merged_provider_fields = tool_provider_fields.or_else(|| {
            tool.function
                .as_ref()
                .and_then(|function| function.provider_specific_fields.as_ref())
                .filter(|value| value.is_truthy())
        });
        let provider_fields = merged_provider_fields.and_then(JsonValue::as_object);
        if let Some(fields) = provider_fields.filter(|fields| !fields.is_empty()) {
            self.provider_fields
                .get_or_insert_with(BTreeMap::new)
                .extend(fields.clone());
        }
    }
}

fn assemble_function(chunks: &[Chunk]) -> Result<AssemblyOutput, AssemblyError> {
    let first = chunks.first().ok_or(AssemblyError::EmptyFunctionInput)?;
    let choice = first
        .choices
        .first()
        .ok_or(AssemblyError::MissingFirstChoice)?;
    let call = choice
        .delta
        .function_call
        .as_ref()
        .ok_or(AssemblyError::MissingFirstFunction)?;
    let name = call
        .name
        .clone()
        .ok_or(AssemblyError::MissingFirstFunctionName)?;
    let mut arguments = String::new();
    for chunk in chunks {
        for current_choice in &chunk.choices {
            append_function_arguments(&mut arguments, current_choice)?;
        }
    }
    Ok(AssemblyOutput::Function(FunctionOutput { name, arguments }))
}

fn append_function_arguments(arguments: &mut String, choice: &Choice) -> Result<(), AssemblyError> {
    let Some(call) = choice.delta.function_call.as_ref() else {
        return Ok(());
    };
    let value = call
        .arguments
        .as_ref()
        .ok_or(AssemblyError::InvalidArguments)?;
    let JsonValue::String(text) = value else {
        return Err(AssemblyError::InvalidArguments);
    };
    arguments.push_str(text);
    Ok(())
}

impl JsonValue {
    fn is_truthy(&self) -> bool {
        match self {
            Self::Null => false,
            Self::Bool(value) => *value,
            Self::Number(JsonNumber::Integer(value)) => *value != 0,
            Self::Number(JsonNumber::Float(value)) => *value != 0.0,
            Self::String(value) => !value.is_empty(),
            Self::Array(value) => !value.is_empty(),
            Self::Object(value) => !value.is_empty(),
        }
    }

    const fn as_object(&self) -> Option<&BTreeMap<String, Self>> {
        if let Self::Object(fields) = self {
            Some(fields)
        } else {
            None
        }
    }
}
