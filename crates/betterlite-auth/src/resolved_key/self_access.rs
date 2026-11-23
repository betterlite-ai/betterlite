use crate::resolved_key::auth::{UserAPIKeyAuth, user_api_key_has_admin_view};

mod tests;

#[must_use]
pub fn allowed_inside_route(auth: &UserAPIKeyAuth, requested_user_id: Option<&str>) -> bool {
    if user_api_key_has_admin_view(auth) {
        return true;
    }
    let Some(requested) = requested_user_id else {
        return false;
    };
    auth.view.token.user_id.as_deref() == Some(requested)
}
