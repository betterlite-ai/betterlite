use betterlite_models::decisions::Json as ContractJson;
use betterlite_prompts::arguments::json::Json;

// B569 and B2201 read and write the decisions JSON, while the Messages request body uses the prompts JSON.
pub(crate) fn to_contract(value: &Json) -> ContractJson {
    match value {
        Json::Null => ContractJson::Null,
        Json::Bool(flag) => ContractJson::Bool(*flag),
        Json::Number(number) => ContractJson::Number(number.clone()),
        Json::Text(text) => ContractJson::Text(text.clone()),
        Json::List(items) => ContractJson::List(items.iter().map(to_contract).collect()),
        Json::Object(fields) => ContractJson::Object(
            fields
                .iter()
                .map(|(key, nested)| (key.clone(), to_contract(nested)))
                .collect(),
        ),
    }
}

pub(crate) fn from_contract(value: ContractJson) -> Json {
    match value {
        ContractJson::Null => Json::Null,
        ContractJson::Bool(flag) => Json::Bool(flag),
        ContractJson::Number(number) => Json::Number(number),
        ContractJson::Text(text) => Json::Text(text),
        ContractJson::List(items) => Json::List(items.into_iter().map(from_contract).collect()),
        ContractJson::Object(fields) => Json::Object(
            fields
                .into_iter()
                .map(|(key, nested)| (key, from_contract(nested)))
                .collect(),
        ),
    }
}
