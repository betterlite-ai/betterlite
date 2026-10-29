use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::Number;

#[derive(Debug, Clone, Deserialize, Default)]
pub struct OffPeakBlock {
    #[serde(default)]
    pub hours_utc: Option<HoursField>,
    #[serde(default)]
    pub windows: Option<WindowsField>,
    #[serde(default)]
    pub weekday_timezone: Option<TimezoneField>,
    #[serde(default)]
    pub input_cost_per_token: Option<RateField>,
    #[serde(default)]
    pub output_cost_per_token: Option<RateField>,
    #[serde(default, rename = "output_cost_per_reasoning_token")]
    pub reasoning_token_cost: Option<RateField>,
    #[serde(default)]
    pub cache_read_input_token_cost: Option<RateField>,
    #[serde(default, rename = "cache_creation_input_token_cost")]
    pub cache_creation_cost: Option<RateField>,
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
    Rule(WindowRule),
    Other(IgnoredAny),
}

#[derive(Debug, Clone, Deserialize)]
pub struct WindowRule {
    #[serde(default)]
    pub hours_utc: Option<HoursField>,
    #[serde(default)]
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
