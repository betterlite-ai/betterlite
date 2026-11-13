use betterlite_server::mcp::outbound_credentials::BOUND_PREFIX;
use betterlite_server::mcp::outbound_credentials::cache_codec::OAuthTokenCacheCodec;
use serde_json::Value;

use super::{parse_token, text, value_error, view};
use crate::helpers::HelperResult;

pub(super) fn fake_codec() -> OAuthTokenCacheCodec {
    OAuthTokenCacheCodec {
        encrypt: Box::new(|value: &str| format!("enc:{value}")),
        decrypt: Box::new(|blob: &str| blob.strip_prefix("enc:").map(str::to_owned)),
    }
}

pub(super) fn codec(input: &Value) -> HelperResult {
    let codec = fake_codec();
    if input.get("operation").and_then(Value::as_str) != Some("encode") {
        let plaintext = text(input, "plaintext")?;
        return Ok(serde_json::json!({
            "decoded": codec.decode(&(codec.encrypt)(plaintext)).as_ref().map(view),
        }));
    }
    let minted = parse_token(input.get("token").unwrap_or(&Value::Null))?;
    let encoded = codec
        .encode(&minted)
        .map_err(|error| value_error(error.to_string()))?;
    let plaintext = (codec.decrypt)(&encoded);
    Ok(serde_json::json!({
        "bound": plaintext.is_some_and(|text| text.starts_with(BOUND_PREFIX)),
        "decoded": codec.decode(&encoded).as_ref().map(view),
    }))
}
