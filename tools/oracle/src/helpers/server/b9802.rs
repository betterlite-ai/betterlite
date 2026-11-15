use betterlite_config::sections::detailed_timing::DetailedTiming;
use betterlite_server::custom_headers::custom_headers;
use betterlite_server::custom_headers::input::CustomHeadersInput;

use crate::helpers::{Helper, HelperResult, OrHelperError as _};

pub(crate) struct Op;

impl Helper for Op {
    type Input = CustomHeadersInput;

    fn run(input: &Self::Input) -> HelperResult {
        let headers = custom_headers(input, DetailedTiming { enabled: false });
        let mut object = serde_json::Map::new();
        for (name, value) in &headers {
            let text = value.to_str().or_value_error()?;
            object.insert(
                name.as_str().to_owned(),
                serde_json::Value::String(text.to_owned()),
            );
        }
        Ok(serde_json::Value::Object(object))
    }
}
