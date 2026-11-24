use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::http::header::CONTENT_TYPE;
use axum::response::{IntoResponse as _, Response};

use super::LoginPageState;
use crate::login_form::LoginPageOptions;

const HTML_CONTENT_TYPE: &str = "text/html; charset=utf-8";

pub(super) async fn fallback_login(State(login): State<Arc<LoginPageState>>) -> Response {
    let html = login.form.html(LoginPageOptions {
        show_deprecation_banner: false,
        hide_default_credentials_hint: login.hide_default_credentials_hint,
    });
    (StatusCode::OK, [(CONTENT_TYPE, HTML_CONTENT_TYPE)], html).into_response()
}
