mod template;
mod tests;

use betterlite_config::sections::login_form::Settings;
use secrecy::ExposeSecret as _;

use crate::custom_url::get_custom_url;

const ACTION_MARKER: &str = "@@ACTION@@";
const BANNER_MARKER: &str = "@@BANNER@@";
const INFO_BOX_MARKER: &str = "@@INFO_BOX@@";
const NEW_LOGIN_URL_MARKER: &str = "@@NEW_LOGIN_URL@@";
const LOGIN_ROUTE: &str = "/login";
const NEW_LOGIN_ROUTE: &str = "ui/login";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginForm {
    pub action: String,
    pub new_login_url: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LoginPageOptions {
    pub show_deprecation_banner: bool,
    pub hide_default_credentials_hint: bool,
}

impl LoginForm {
    #[must_use]
    pub fn from_settings(settings: &Settings) -> Self {
        Self {
            action: format!(
                "{}{}{LOGIN_ROUTE}",
                settings.proxy_base_url, settings.server_root_path
            ),
            new_login_url: get_custom_url(
                Some(settings.proxy_base_url.as_str()),
                "",
                settings.server_root_path.as_str(),
                Some(NEW_LOGIN_ROUTE),
            ),
        }
    }

    #[must_use]
    pub fn html(&self, options: LoginPageOptions) -> String {
        let banner = if options.show_deprecation_banner {
            template::BANNER.replace(NEW_LOGIN_URL_MARKER, &escape_attribute(&self.new_login_url))
        } else {
            String::new()
        };
        let info_box = if options.hide_default_credentials_hint {
            String::new()
        } else {
            template::INFO_BOX.to_owned()
        };
        template::PAGE
            .replace(ACTION_MARKER, &escape_attribute(&self.action))
            .replace(BANNER_MARKER, &banner)
            .replace(INFO_BOX_MARKER, &info_box)
    }
}

#[must_use]
pub fn should_hide_credentials_hint(settings: &Settings) -> bool {
    let setting_hides = settings.hide_default_credentials_hint == Some(true);
    // LiteLLM lowercases the raw text and compares it with exactly "true", so " TRUE " and "1" stay false
    let environment_hides = settings
        .hide_hint_environment
        .as_deref()
        .is_some_and(|value| value.to_lowercase() == "true");
    let password_hides = settings
        .ui_password
        .as_ref()
        .is_some_and(|password| !password.expose_secret().is_empty());
    setting_hides || environment_hides || password_hides
}

fn escape_attribute(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#x27;"),
            other => escaped.push(other),
        }
    }
    escaped
}
