use http::Method;

use super::api::ApiError;
use super::client::KeysClient;
use super::output;
use super::wire::KeyDeleteBody;
use super::{DeleteKeys, KeysError, KeysPorts, split_commas};

const DELETE_PATH: &str = "/key/delete";

pub(super) fn delete_keys(options: &DeleteKeys, ports: KeysPorts<'_>) -> Result<(), KeysError> {
    let KeysPorts {
        context,
        client,
        redactor,
        stdout,
        stderr,
    } = ports;
    let request = KeyDeleteBody {
        keys: split_commas(options.keys.as_deref()),
        key_aliases: split_commas(options.key_aliases.as_deref()),
    };
    let encoded = serde_json::to_vec(&request).map_err(KeysError::Json)?;
    let api = KeysClient {
        transport: client,
        base_url: context.base_url.trim_end_matches('/'),
        api_key: context.api_key.as_ref(),
        redactor,
    };
    match api.send(Method::POST, DELETE_PATH, &[], Some(encoded)) {
        Ok(reply) => output::write_json(&reply.value, output::JsonText::Pretty, stdout)
            .map_err(KeysError::Json),
        Err(ApiError::Http { status, body, .. }) => {
            output::print_http_error(status, &body, stdout, stderr).map_err(KeysError::Json)?;
            Err(KeysError::Aborted)
        }
        Err(error) => Err(KeysError::Client(error)),
    }
}
