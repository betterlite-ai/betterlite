use serde::de::DeserializeOwned;

use crate::payloads::last_json_object;

pub fn parse_output<T>(output_json: Option<&str>, text: &str) -> (Option<T>, Option<String>)
where
    T: DeserializeOwned,
{
    let Some(raw) = output_json
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| last_json_object(text))
    else {
        return (
            None,
            Some("no JSON object found in the final answer".to_owned()),
        );
    };
    match serde_json::from_str(&raw) {
        Ok(parsed) => (Some(parsed), None),
        Err(error) => (None, Some(error.to_string())),
    }
}
