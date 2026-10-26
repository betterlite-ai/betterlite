mod tests;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UserApiKeyAuth {
    pub user_role: Option<String>,
}

#[must_use]
pub fn user_api_key_has_admin_view(api_key: &UserApiKeyAuth) -> bool {
    matches!(
        api_key.user_role.as_deref(),
        Some("proxy_admin" | "proxy_admin_viewer")
    )
}
