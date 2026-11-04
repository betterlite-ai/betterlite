#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    status: u16,
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let message = format!(
            "Upstream realtime handshake rejected with HTTP {}",
            input.status
        );
        let close_code = match input.status {
            401 | 403 => 1008,
            429 => 1013,
            _ => 1011,
        };
        Ok(serde_json::json!({
            "event": {"type": "error", "error": {"type": "server_error", "message": message}},
            "close_code": close_code,
            "close_reason": message,
        }))
    }
}
