use betterlite_prompts::arguments::json::Object;

use crate::purview_dlp_evaluation::error_handling::{self, PurviewDlpHttpError};
use crate::purview_graph_client::PurviewGuardrailBase;
use crate::purview_graph_client::user_id_resolution::{
    KeyIdentity, ResolvedUserId, resolve_user_id,
};

pub const UNTRUSTED_IDENTITY_ERROR: &str = "Microsoft Purview DLP: No proxy-authenticated user identity; bind user_id to the API key (caller-supplied metadata cannot be used for blocking DLP)";
pub const NO_IDENTITY_ERROR: &str = "Microsoft Purview DLP: No proxy-authenticated user identity; bind user_id to the API key for blocking DLP";

pub fn resolve_user_id_for_blocking(
    base: &PurviewGuardrailBase,
    data: &Object,
    identity: &KeyIdentity<'_>,
) -> Result<String, PurviewDlpHttpError> {
    match resolve_user_id(base, data, identity) {
        Some(ResolvedUserId::Trusted(user_id)) => Ok(user_id),
        Some(ResolvedUserId::Untrusted(_)) => {
            Err(error_handling::message_error(UNTRUSTED_IDENTITY_ERROR))
        }
        None => Err(error_handling::message_error(NO_IDENTITY_ERROR)),
    }
}

mod tests;
