use betterlite_models::decisions::Json as ContractJson;
use betterlite_models::prompt_contracts::PromptParams;

use crate::arguments::json::{Json, Object};

// The dotprompt renderer and BitBucket's template manager read the prompts crate's Json type.
pub(crate) fn to_prompt_object(params: &PromptParams) -> Object {
    let mut object = Object::new();
    for (key, value) in params {
        object.insert(key.clone(), to_prompt_json(value));
    }
    object
}

fn to_prompt_json(value: &ContractJson) -> Json {
    match value {
        ContractJson::Null => Json::Null,
        ContractJson::Bool(flag) => Json::Bool(*flag),
        ContractJson::Number(number) => Json::Number(number.clone()),
        ContractJson::Text(text) => Json::Text(text.clone()),
        ContractJson::List(items) => Json::List(items.iter().map(to_prompt_json).collect()),
        ContractJson::Object(fields) => Json::Object(to_prompt_object(fields)),
    }
}
