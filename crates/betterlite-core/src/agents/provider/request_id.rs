use super::AgentProviderError;
use crate::agents::json::AgentJson;
use std::fmt::Write;

pub(super) fn python_string(value: &AgentJson) -> Result<String, AgentProviderError> {
    match value {
        AgentJson::Null => Ok(String::from("None")),
        AgentJson::Bool(true) => Ok(String::from("True")),
        AgentJson::Bool(false) => Ok(String::from("False")),
        AgentJson::Number(number) => Ok(python_number(number)),
        AgentJson::String(text) => quoted(text),
        AgentJson::Array(values) => {
            let items = values
                .iter()
                .map(python_string)
                .collect::<Result<Vec<_>, _>>()?
                .join(", ");
            Ok(format!("[{items}]"))
        }
        AgentJson::Object(values) => {
            let items = values
                .iter()
                .map(|(key, item)| Ok(format!("{}: {}", quoted(key)?, python_string(item)?)))
                .collect::<Result<Vec<_>, _>>()?
                .join(", ");
            Ok(format!("{{{items}}}"))
        }
    }
}

fn python_number(number: &serde_json::Number) -> String {
    let spelling = number.to_string();
    if !spelling.contains(['.', 'e', 'E']) {
        return spelling;
    }
    let Some(value) = number.as_f64() else {
        return spelling;
    };
    let debug = format!("{value:?}");
    match debug.split_once('e') {
        Some((mantissa, exponent)) => {
            let (sign, digits) = exponent
                .strip_prefix('-')
                .map_or(('+', exponent), |rest| ('-', rest));
            format!("{mantissa}e{sign}{digits:0>2}")
        }
        None => debug,
    }
}

fn quoted(text: &str) -> Result<String, AgentProviderError> {
    let quote = if text.contains('\'') && !text.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut result = String::from(quote);
    for character in text.chars() {
        match character {
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            control if control.is_control() => {
                Write::write_fmt(&mut result, format_args!("\\x{:02x}", u32::from(control)))
                    .map_err(|source| AgentProviderError::Format { source })?;
            }
            delimiter if delimiter == quote => {
                result.push('\\');
                result.push(delimiter);
            }
            other => result.push(other),
        }
    }
    result.push(quote);
    Ok(result)
}
