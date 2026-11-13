use betterlite_models::decisions::Json as ContractJson;

use crate::arguments::json::Json;

impl From<&Json> for ContractJson {
    fn from(value: &Json) -> Self {
        match value {
            Json::Null => Self::Null,
            Json::Bool(flag) => Self::Bool(*flag),
            Json::Number(number) => Self::Number(number.clone()),
            Json::Text(text) => Self::Text(text.clone()),
            Json::List(items) => Self::List(items.iter().map(Self::from).collect()),
            Json::Object(fields) => Self::Object(
                fields
                    .iter()
                    .map(|(key, field)| (key.clone(), Self::from(field)))
                    .collect(),
            ),
        }
    }
}
