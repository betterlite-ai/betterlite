use rust_decimal::Decimal;
use serde::Deserialize;
use serde::de::IgnoredAny;

// Python's str.strip also removes the separators U+001C to U+001F, which Rust's trim keeps
const PYTHON_SPACE: fn(char) -> bool =
    |character| character.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&character);
const BOOL_WORDS: [(&str, bool); 12] = [
    ("0", false),
    ("off", false),
    ("f", false),
    ("false", false),
    ("n", false),
    ("no", false),
    ("1", true),
    ("on", true),
    ("t", true),
    ("true", true),
    ("y", true),
    ("yes", true),
];

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum OptionValue {
    Bool(bool),
    Integer(i64),
    Float(f64),
    Text(String),
    Other(IgnoredAny),
}

impl OptionValue {
    pub(crate) fn as_bool(&self) -> Option<bool> {
        // pydantic takes a bool, int or whole float as a bool only when it equals 0 or 1
        let word = match self {
            Self::Text(text) => text.trim_matches(PYTHON_SPACE).to_lowercase(),
            Self::Bool(_) | Self::Integer(_) | Self::Float(_) | Self::Other(_) => {
                self.as_integer()?.to_string()
            }
        };
        let spelled = BOOL_WORDS.iter().find(|(spelling, _)| *spelling == word);
        spelled.map(|(_, flag)| *flag)
    }

    pub(crate) fn as_integer(&self) -> Option<i64> {
        match self {
            Self::Bool(flag) => Some(i64::from(*flag)),
            Self::Integer(number) => Some(*number),
            Self::Float(number) => integer_from_float(*number),
            Self::Text(text) => integer_from_text(text),
            Self::Other(_) => None,
        }
    }

    pub(crate) const fn type_name(&self) -> &'static str {
        match self {
            Self::Bool(_) => "bool",
            Self::Integer(_) => "int",
            Self::Float(_) => "float",
            Self::Text(_) => "str",
            Self::Other(_) => "collection",
        }
    }
}

fn integer_from_float(number: f64) -> Option<i64> {
    let exact = Decimal::from_f64_retain(number)?;
    if !exact.fract().is_zero() {
        return None;
    }
    i64::try_from(exact).ok()
}

pub(crate) fn integer_from_text(text: &str) -> Option<i64> {
    let stripped = text.trim_matches(PYTHON_SPACE);
    let whole = match stripped.split_once('.') {
        None => stripped,
        Some((integer_part, zeros))
            if !zeros.is_empty() && zeros.bytes().all(|byte| byte == b'0') =>
        {
            integer_part
        }
        Some(_) => return None,
    };
    let digits = whole.strip_prefix(['+', '-']).unwrap_or(whole);
    if digits.starts_with('_') || digits.ends_with('_') || digits.contains("__") {
        return None;
    }
    whole.replace('_', "").parse().ok()
}
