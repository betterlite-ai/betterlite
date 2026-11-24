use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_auth::user_roles::UserRole;
use betterlite_lens::models::settings::LensSettings;

use crate::lens_access::error::LensAccessError;
use crate::lens_access::scope::key_caller;
use crate::lens_access::selection::validate_selection;
use crate::lens_access::state::LensRoutesState;

const ALL_PROXY_MODELS: &str = "all-proxy-models";

pub(crate) fn validate_model<Db, Store>(
    settings: &LensSettings,
    auth: &UserAPIKeyAuth,
    state: &LensRoutesState<Db, Store>,
) -> Result<(), LensAccessError> {
    validate_selection(settings)?;
    let Some(models) = state.models.as_ref() else {
        return Err(LensAccessError::UnknownModel);
    };
    let caller = key_caller(auth)?;
    let known = models.get_model_names(caller.team_id.as_deref());
    if !known.iter().any(|name| name == &settings.model) {
        return Err(LensAccessError::UnknownModel);
    }
    let allowed = caller.models;
    let holds_model = allowed
        .iter()
        .any(|name| name == &settings.model || name == ALL_PROXY_MODELS);
    if caller.user_role != Some(UserRole::ProxyAdmin) && !allowed.is_empty() && !holds_model {
        return Err(LensAccessError::ModelDenied);
    }
    Ok(())
}

mod tests;
