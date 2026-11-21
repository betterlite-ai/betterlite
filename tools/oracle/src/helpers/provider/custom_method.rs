use std::collections::BTreeMap;

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_provider::legacy_dispatch::custom::custom_chat_llm_router;

#[derive(Debug, serde::Deserialize)]
pub(crate) struct Input {
    cases: BTreeMap<String, Call>,
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct Call {
    async_fn: bool,
    #[serde(default)]
    stream: Option<bool>,
}

pub(crate) struct Op;

// There is no request here: the selector reads only the async flag and the stream flag
impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut selected = serde_json::Map::new();
        for (name, call) in &input.cases {
            let method = custom_chat_llm_router(call.async_fn, call.stream);
            selected.insert(
                name.clone(),
                serde_json::Value::String(method.name().to_owned()),
            );
        }
        Ok(serde_json::Value::Object(selected))
    }
}
