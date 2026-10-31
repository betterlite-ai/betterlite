use axum::http::StatusCode;
use axum::response::{Html, IntoResponse as _, Response};

mod tests;

const PAGE_SHELL: &str = include_str!("admin_setup/page_shell.html");
const MISSING_KEYS_BODY: &str = include_str!("admin_setup/missing_keys_body.html");
const ADMIN_UI_DISABLED_BODY: &str = include_str!("admin_setup/admin_ui_disabled_body.html");

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
    let body = MISSING_KEYS_BODY.replacen("@@MISSING_KEYS@@", missing_key_names, 1);
    render_page(
        "Environment Setup Instructions",
        "Environment Setup Instructions",
        &body,
    )
}

#[must_use]
pub fn admin_ui_disabled() -> Response {
    let page = render_page(
        "Admin UI Disabled",
        "Admin UI is Disabled",
        ADMIN_UI_DISABLED_BODY,
    );
    (StatusCode::OK, Html(page)).into_response()
}

fn render_page(title: &str, heading: &str, body: &str) -> String {
    let with_title = PAGE_SHELL.replacen("@@TITLE@@", title, 1);
    let with_heading = with_title.replacen("@@HEADING@@", heading, 1);
    with_heading.replacen("@@BODY@@", body, 1)
}
