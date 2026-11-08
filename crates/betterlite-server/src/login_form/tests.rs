#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::login_form::Settings;
use rstest::rstest;
use secrecy::SecretString;

use super::{LoginForm, LoginPageOptions, should_hide_credentials_hint};

fn settings(proxy_base_url: &str, server_root_path: &str) -> Settings {
    Settings {
        proxy_base_url: proxy_base_url.to_owned(),
        server_root_path: server_root_path.to_owned(),
        hide_default_credentials_hint: false,
        hide_hint_environment: "false".to_owned(),
        ui_password: SecretString::from(String::new()),
    }
}

fn configured(source: &str, environment: &[(&str, &str)]) -> Settings {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }
        .load(source)
        .unwrap()
        .sections
        .login_form
}

fn render(settings: &Settings) -> String {
    LoginForm::from_settings(settings).html(LoginPageOptions {
        show_deprecation_banner: false,
        hide_default_credentials_hint: should_hide_credentials_hint(settings),
    })
}

#[test]
fn configured_bindings_reach_the_renderer() {
    let settings = configured(
        "general_settings: {hide_default_credentials_hint: true}",
        &[
            ("PROXY_BASE_URL", "https://proxy.example.com/base/"),
            ("SERVER_ROOT_PATH", "/gateway"),
        ],
    );
    let form = LoginForm::from_settings(&settings);
    assert_eq!(form.action, "https://proxy.example.com/base//gateway/login");
    assert_eq!(
        form.new_login_url,
        "https://proxy.example.com/base/gateway/ui/login"
    );
    assert!(should_hide_credentials_hint(&settings));
    let html = form.html(LoginPageOptions {
        show_deprecation_banner: false,
        hide_default_credentials_hint: should_hide_credentials_hint(&settings),
    });
    assert!(html.contains(
        r#"<form action="https://proxy.example.com/base//gateway/login" method="post">"#
    ));
    assert!(!html.contains(r#"<div class="info-box">"#));
}

#[rstest]
#[case::environment_on(
    "general_settings: {hide_default_credentials_hint: false}",
    &[("LITELLM_HIDE_DEFAULT_CREDENTIALS_HINT", "true")]
)]
#[case::password_on(
    "general_settings: {hide_default_credentials_hint: false}",
    &[("UI_PASSWORD", "s3cret")]
)]
fn false_setting_does_not_cancel_a_true_source(
    #[case] source: &str,
    #[case] environment: &[(&str, &str)],
) {
    let settings = configured(source, environment);
    assert!(should_hide_credentials_hint(&settings));
    assert!(!render(&settings).contains(r#"<div class="info-box">"#));
}

#[test]
fn default_form() {
    assert_eq!(
        LoginPageOptions::default(),
        LoginPageOptions {
            show_deprecation_banner: false,
            hide_default_credentials_hint: false,
        }
    );
    let html = LoginForm::from_settings(&settings("", "")).html(LoginPageOptions::default());
    assert!(html.contains(r#"<form action="/login" method="post">"#));
    assert!(!html.contains(r#"<div class="deprecation-banner">"#));
    assert!(html.contains(r#"<div class="info-box">"#));
    assert!(html.contains("<title>betterlite Login</title>"));
}

#[test]
fn banner_link() {
    let html = LoginForm::from_settings(&settings("https://proxy.example.com/base", "/gateway"))
        .html(LoginPageOptions {
            show_deprecation_banner: true,
            hide_default_credentials_hint: false,
        });
    assert!(html.contains(r#"<div class="deprecation-banner">"#));
    assert!(html.contains(
        r#"<a href="https://proxy.example.com/base/gateway/ui/login">new login page</a>"#
    ));
}

#[rstest]
#[case::visible(false, "false", None, false)]
#[case::setting_on(true, "false", None, true)]
#[case::environment_on(false, "TRUE", None, true)]
#[case::environment_spaced(false, " TRUE ", None, false)]
#[case::environment_numeric(false, "1", None, false)]
#[case::password_on(false, "false", Some("s3cret"), true)]
#[case::password_empty(false, "false", Some(""), false)]
fn hidden_hint(
    #[case] setting: bool,
    #[case] environment: &str,
    #[case] password: Option<&str>,
    #[case] hidden: bool,
) {
    let mut sources = settings("", "");
    sources.hide_default_credentials_hint = setting;
    sources.hide_hint_environment = environment.to_owned();
    sources.ui_password = SecretString::from(password.unwrap_or_default().to_owned());
    assert_eq!(should_hide_credentials_hint(&sources), hidden);
    let html = LoginForm::from_settings(&sources).html(LoginPageOptions {
        show_deprecation_banner: false,
        hide_default_credentials_hint: hidden,
    });
    assert_eq!(!html.contains(r#"<div class="info-box">"#), hidden);
    assert_eq!(html.contains("<code>MASTER_KEY</code>"), !hidden);
}

#[test]
fn form_fields() {
    let html = LoginForm::from_settings(&settings("", "")).html(LoginPageOptions::default());
    assert!(
        html.contains(r#"<label for="username">Username<span class="required">*</span></label>"#)
    );
    assert!(
        html.contains(r#"<label for="password">Password<span class="required">*</span></label>"#)
    );
    assert!(html.contains(
        r#"<input type="text" id="username" name="username" required placeholder="Enter your username" autocomplete="username">"#
    ));
    assert!(html.contains(
        r#"<input type="password" id="password" name="password" required placeholder="Enter your password" autocomplete="current-password">"#
    ));
    assert!(html.contains(r#"<input type="submit" value="Login">"#));
}

#[test]
fn toggle_password() {
    let html = LoginForm::from_settings(&settings("", "")).html(LoginPageOptions::default());
    assert!(html.contains(
        r#"<input type="checkbox" id="show-password" onclick="togglePasswordVisibility()">"#
    ));
    assert!(html.contains(r#"<label for="show-password">Show password</label>"#));
    assert!(html.contains("function togglePasswordVisibility() {"));
    assert!(html.contains(
        r#"passwordField.type = passwordField.type === "password" ? "text" : "password";"#
    ));
}

#[test]
fn retained_styles() {
    let html = LoginForm::from_settings(&settings("", "")).html(LoginPageOptions::default());
    assert!(html.contains(
        "font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, Cantarell, sans-serif;"
    ));
    assert!(html.contains("background-color: #6466E9;"));
    assert!(html.contains("background-color: #4138C2;"));
    assert!(html.contains("border-left: 4px solid #2563eb;"));
    assert!(html.contains("background-color: #fee2e2;"));
    assert!(html.contains("border: 1px solid #ef4444;"));
    assert!(html.contains("color: #991b1b;"));
    assert!(html.contains("box-shadow: 0 4px 12px rgba(0, 0, 0, 0.1);"));
}

#[test]
fn rebranded_login() {
    let html = LoginForm::from_settings(&settings("", "")).html(LoginPageOptions::default());
    assert!(!html.contains("LiteLLM"));
    assert!(html.contains("<title>betterlite Login</title>"));
    assert!(html.contains("&#128645; betterlite"));
    assert!(html.contains("Access your betterlite Admin UI."));
    assert!(html.contains("Password is your set betterlite Proxy <code>MASTER_KEY</code>."));
    assert!(html.contains("<code>admin</code>"));
    assert!(html.contains(r#"href="https://docs.litellm.ai/docs/proxy/ui""#));
}

#[test]
fn escaped_urls() {
    let html = LoginForm::from_settings(&settings(
        "https://proxy.example.com/a?x=\"y\"&z=<w>'v",
        "/gateway",
    ))
    .html(LoginPageOptions {
        show_deprecation_banner: true,
        hide_default_credentials_hint: false,
    });
    assert!(html.contains(
        r#"<form action="https://proxy.example.com/a?x=&quot;y&quot;&amp;z=&lt;w&gt;&#x27;v/gateway/login" method="post">"#
    ));
    assert!(html.contains(
        r#"<a href="https://proxy.example.com/a?x=&quot;y&quot;&amp;z=&lt;w&gt;&#x27;v/gateway/ui/login">new login page</a>"#
    ));
    assert!(!html.contains(r#"action="https://proxy.example.com/a?x="y""#));
}
