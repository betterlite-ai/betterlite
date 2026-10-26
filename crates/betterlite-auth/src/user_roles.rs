mod tests;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UserApiKeyAuth {
    pub user_role: Option<String>,
}

#[must_use]
pub fn user_api_key_has_admin_view(api_key: &UserApiKeyAuth) -> bool {
    let role = api_key.user_role.as_deref();
    role.is_some_and(|value| value == "proxy_admin" || value == "proxy_admin_viewer")
}
