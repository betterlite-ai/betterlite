use betterlite_models::decisions::Json;

use crate::api::request_metadata_safety::MetadataPrincipal;

pub const ALLOW_CLIENT_MOCK_RESPONSE_KEY: &str = "allow_client_mock_response";
pub const ALLOW_CLIENT_MESSAGE_REDACTION_OPT_OUT_KEY: &str =
    "allow_client_message_redaction_opt_out";
pub const ALLOW_CLIENT_PRICING_OVERRIDE_KEY: &str = "allow_client_pricing_override";

// the flag is an admin-set boolean on the key or the team; a string or a number does not opt in
#[must_use]
pub fn key_or_team_metadata_flag_is_true(
    principal: &MetadataPrincipal,
    metadata_key: &str,
) -> bool {
    [&principal.metadata, &principal.team_metadata]
        .into_iter()
        .flatten()
        .any(|metadata| matches!(metadata.get(metadata_key), Some(Json::Bool(true))))
}
