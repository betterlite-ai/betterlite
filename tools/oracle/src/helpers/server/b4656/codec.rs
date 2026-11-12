use betterlite_server::mcp::outbound_credentials::cache_codec::OAuthTokenCacheCodec;
use betterlite_server::mcp::outbound_credentials::{BOUND_PREFIX, OAuthToken};
use serde::Deserialize;

use super::support::{TokenInput, value_error, view};
use crate::helpers::HelperResult;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Encode { token: TokenInput },
    Decode { plaintext: String },
    DecodeBlob { blob: String },
}

pub(super) fn run(input: &Input) -> HelperResult {
    let codec = OAuthTokenCacheCodec {
        encrypt: Box::new(|value: &str| format!("enc:{value}")),
        decrypt: Box::new(|blob: &str| blob.strip_prefix("enc:").map(str::to_owned)),
    };
    match input {
        Input::Encode { token } => {
            let minted: OAuthToken = token.to_token()?;
            let encoded = codec
                .encode(&minted)
                .map_err(|error| value_error(error.to_string()))?;
            let bound = (codec.decrypt)(&encoded)
                .is_some_and(|plaintext| plaintext.starts_with(BOUND_PREFIX));
            Ok(serde_json::json!({
                "bound": bound,
                "decoded": codec.decode(&encoded).as_ref().map(view),
            }))
        }
        Input::Decode { plaintext } => Ok(serde_json::json!({
            "decoded": codec.decode(&format!("enc:{plaintext}")).as_ref().map(view),
        })),
        Input::DecodeBlob { blob } => Ok(serde_json::json!({
            "decoded": codec.decode(blob).as_ref().map(view),
        })),
    }
}
