use axum::http::StatusCode;
use axum::response::{Html, IntoResponse as _, Response};

mod tests;

const MISSING_KEYS_TEMPLATE: &str = include_str!("admin_setup/missing_keys_page.html");
const ADMIN_UI_DISABLED_TEMPLATE: &str = include_str!("admin_setup/admin_ui_disabled_page.html");

#[derive(Clone, Copy)]
pub struct ProxyAvailability {
    pub has_master_key: bool,
    pub has_database: bool,
}

#[must_use]
pub fn show_missing_vars_in_env(availability: ProxyAvailability) -> Option<Response> {
    let missing_key_names = match (availability.has_master_key, availability.has_database) {
        (true, true) => return None,
        (false, false) => "DATABASE_URL, LITELLM_MASTER_KEY",
        (true, false) => "DATABASE_URL",
        (false, true) => "LITELLM_MASTER_KEY",
    };
    Some(Html(missing_keys_form(missing_key_names)).into_response())
}

#[must_use]
pub fn missing_keys_form(missing_key_names: &str) -> String {
    // LiteLLM's format call unescapes CSS braces and adds the closing quote indentation.
    format!(
        "\n{}    ",
        MISSING_KEYS_TEMPLATE
            .replace("{{", "{")
            .replace("}}", "}")
            .replace("@@MISSING_KEYS@@", missing_key_names)
    )
}

#[must_use]
pub fn admin_ui_disabled() -> Response {
    let page = format!(
        "\n{}    ",
        ADMIN_UI_DISABLED_TEMPLATE
            .replace("{{", "{")
            .replace("}}", "}")
    );
    (StatusCode::OK, Html(page)).into_response()
}
