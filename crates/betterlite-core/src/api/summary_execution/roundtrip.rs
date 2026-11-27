use serde::Serialize;
use serde::de::DeserializeOwned;

// the prompts Json and the decisions Json are separate types, so one crosses to the other through serde
pub fn convert<Value, Target>(value: &Value) -> Result<Target, serde_json::Error>
where
    Value: Serialize,
    Target: DeserializeOwned,
{
    serde_json::to_value(value).and_then(serde_json::from_value)
}
