use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::Number;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct OffPeakBlock {
    pub hours_utc: Option<HoursField>,
    pub windows: Option<WindowsField>,
    pub weekday_timezone: Option<TimezoneField>,
    pub input_cost_per_token: Option<RateField>,
    pub output_cost_per_token: Option<RateField>,
    #[serde(rename = "output_cost_per_reasoning_token")]
    pub reasoning_token_cost: Option<RateField>,
    pub cache_read_input_token_cost: Option<RateField>,
    #[serde(rename = "cache_creation_input_token_cost")]
    pub cache_creation_cost: Option<RateField>,
}

// LiteLLM's _open_off_peak_block requires a Mapping, so a list-shaped value is no block at all.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum OffPeakBlockEntry {
    Sequence(Vec<IgnoredAny>),
    Block(Box<OffPeakBlock>),
    Other(IgnoredAny),
}

impl OffPeakBlockEntry {
    #[must_use]
    pub const fn as_block(&self) -> Option<&OffPeakBlock> {
        match self {
            Self::Block(block) => Some(block),
            Self::Sequence(_) | Self::Other(_) => None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum HoursField {
    One(String),
    Many(Vec<StringOrIgnored>),
    Other(IgnoredAny),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum StringOrIgnored {
    Text(String),
    Ignored(IgnoredAny),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum WindowsField {
    List(Vec<WindowRuleEntry>),
    Other(IgnoredAny),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum WindowRuleEntry {
    Sequence(Vec<IgnoredAny>),
    Rule(WindowRule),
    Other(IgnoredAny),
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct WindowRule {
    pub hours_utc: Option<HoursField>,
    pub weekdays: Option<WeekdaysField>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum WeekdaysField {
    List(Vec<WeekdayEntry>),
    Other(IgnoredAny),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum WeekdayEntry {
    Bool(bool),
    Int(i64),
    Text(String),
    Other(IgnoredAny),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum TimezoneField {
    Text(String),
    Other(IgnoredAny),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum RateField {
    Bool(bool),
    Number(Number),
    Text(String),
    Other(IgnoredAny),
}
