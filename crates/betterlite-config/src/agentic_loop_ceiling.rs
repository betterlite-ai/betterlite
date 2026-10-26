use thiserror::Error;

mod tests;

pub const DEFAULT_MAX_AGENTIC_LOOPS: u32 = 3;

#[derive(Debug, Clone, PartialEq)]
pub enum AgenticLoopValue {
    Null,
    Boolean(bool),
    Integer(i64),
    Float(f64),
    Text(String),
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AgenticLoopCeilingError {
    #[error("{field} must be a whole number")]
    NotWholeNumber { field: String },
    #[error("{field} must be at least 1")]
    BelowMinimum { field: String, value: i64 },
}

pub fn validated_max_agentic_loops(
    value: &AgenticLoopValue,
    field: &str,
) -> Result<Option<u32>, AgenticLoopCeilingError> {
    let Some(ceiling) = whole_number(value) else {
        if matches!(value, AgenticLoopValue::Null) {
            return Ok(None);
        }
        return Err(AgenticLoopCeilingError::NotWholeNumber {
            field: field.to_owned(),
        });
    };
    if ceiling < 1 {
        return Err(AgenticLoopCeilingError::BelowMinimum {
            field: field.to_owned(),
            value: ceiling,
        });
    }
    let Ok(bounded_ceiling) = u32::try_from(ceiling) else {
        return Err(AgenticLoopCeilingError::NotWholeNumber {
            field: field.to_owned(),
        });
    };
    Ok(Some(bounded_ceiling))
}

fn whole_number(value: &AgenticLoopValue) -> Option<i64> {
    match value {
        AgenticLoopValue::Boolean(_) | AgenticLoopValue::Null => None,
        AgenticLoopValue::Integer(integer) => Some(*integer),
        AgenticLoopValue::Float(float) => {
            if float.fract() == 0.0 {
                let text = float.to_string();
                return text.parse().ok();
            }
            None
        }
        AgenticLoopValue::Text(text) => {
            let trimmed = text.trim();
            let digits = trimmed.strip_prefix(['+', '-']).unwrap_or(trimmed);
            if !digits
                .split('_')
                .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
            {
                return None;
            }
            trimmed.replace('_', "").parse().ok()
        }
    }
}
