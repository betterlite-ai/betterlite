use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::spend::sessions::cursor::decode_session_cursor;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Input {
    cursor: Option<String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let Some(cursor) = decode_session_cursor(input.cursor.as_deref()) else {
            return Ok(serde_json::json!(null));
        };
        Ok(serde_json::json!([
            cursor.last_activity,
            cursor.session_key,
            cursor.api_key_hash
        ]))
    }
}
