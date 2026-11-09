use indexmap::IndexMap;
use secrecy::ExposeSecret as _;

use betterlite_models::decisions::Json;

use crate::logging_privacy::PrivacyContext;
use crate::logging_privacy::callbacks::TrustedCallbacks;
use crate::logging_privacy::request::PrivacyRequest;

use super::error::DynamicCallbackError;

pub fn standard_callback_params(
    request: &PrivacyRequest,
    trusted: &TrustedCallbacks,
    context: PrivacyContext,
) -> Result<IndexMap<String, Json>, DynamicCallbackError> {
    let mut params = request.dynamic_params(context)?;
    let mut overlay = IndexMap::new();
    trusted.overlay(&mut overlay);
    for (field, value) in overlay {
        params.insert(field, Json::Text(value.expose_secret().to_owned()));
    }
    Ok(params)
}
