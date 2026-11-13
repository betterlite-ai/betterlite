use std::iter::Peekable;
use std::str::Chars;

use betterlite_models::decisions::Json;
use betterlite_models::prompt_contracts::{AllMessageValues, PromptParams};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CompileError {
    #[error("prompt template message {index} is not an object")]
    MessageNotObject { index: usize },
    #[error("prompt variable {name} is missing")]
    MissingVariable { name: String },
    #[error("prompt variable {name} must be a string, number, bool or null")]
    VariableType { name: String },
    #[error("prompt template field {field} uses an unsupported format specifier")]
    UnsupportedField { field: String },
    #[error("prompt template has an unmatched brace")]
    UnmatchedBrace,
}

pub fn compile_prompt(
    prompt_template: &mut [AllMessageValues],
    prompt_variables: Option<&PromptParams>,
) -> Result<(), CompileError> {
    let empty = PromptParams::new();
    let variables = prompt_variables.unwrap_or(&empty);
    for (index, template) in prompt_template.iter_mut().enumerate() {
        let Json::Object(message) = template else {
            return Err(CompileError::MessageNotObject { index });
        };
        if let Some(Json::Text(content)) = message.get_mut("content")
            && !content.is_empty()
        {
            *content = format_content(content, variables)?;
        }
    }
    Ok(())
}

// LiteLLM turns {{name}} into {name} and hands the result to Python's str.format, so doubled braces stay literal.
fn format_content(content: &str, variables: &PromptParams) -> Result<String, CompileError> {
    let converted = content.replace("{{", "{").replace("}}", "}");
    let mut output = String::with_capacity(converted.len());
    let mut characters = converted.chars().peekable();
    while let Some(character) = characters.next() {
        match (character, characters.peek()) {
            ('{', Some('{')) => {
                characters.next();
                output.push('{');
            }
            ('}', Some('}')) => {
                characters.next();
                output.push('}');
            }
            ('{', _) => {
                let name = read_field_name(&mut characters)?;
                output.push_str(&render_variable(variables, &name)?);
            }
            ('}', _) => return Err(CompileError::UnmatchedBrace),
            (other, _) => output.push(other),
        }
    }
    Ok(output)
}

fn read_field_name(characters: &mut Peekable<Chars<'_>>) -> Result<String, CompileError> {
    let mut name = String::new();
    for character in characters.by_ref() {
        match character {
            '}' => {
                if name.contains(['!', ':', '.', '[']) {
                    return Err(CompileError::UnsupportedField { field: name });
                }
                return Ok(name);
            }
            '{' => return Err(CompileError::UnsupportedField { field: name }),
            other => name.push(other),
        }
    }
    Err(CompileError::UnmatchedBrace)
}

fn render_variable(variables: &PromptParams, name: &str) -> Result<String, CompileError> {
    match variables.get(name) {
        None => Err(CompileError::MissingVariable {
            name: name.to_owned(),
        }),
        Some(Json::Text(text)) => Ok(text.clone()),
        Some(Json::Number(number)) => Ok(number.to_string()),
        Some(Json::Bool(flag)) => Ok(flag.to_string()),
        Some(Json::Null) => Ok("null".to_owned()),
        Some(Json::List(_) | Json::Object(_)) => Err(CompileError::VariableType {
            name: name.to_owned(),
        }),
    }
}
