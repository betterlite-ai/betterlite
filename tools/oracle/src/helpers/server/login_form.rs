use std::collections::BTreeMap;

use betterlite_config::sections::Section as _;
use betterlite_config::sections::login_form::Raw;
use betterlite_server::login_form::{LoginForm, LoginPageOptions, should_hide_credentials_hint};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

type Markup<'html> = Result<&'html str, Box<HelperError>>;

const MISSING: &str = "login form markup is missing a boundary";
const DOCS_LINK: &str = "<a href=\"https://docs.litellm.ai/docs/proxy/ui\"";

pub(crate) struct Op;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    #[serde(default)]
    environment: BTreeMap<String, String>,
    general_settings: Value,
    #[serde(default)]
    show_deprecation_banner: bool,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let raw: Raw = serde_json::from_value(json!({
            "general_settings": &input.general_settings,
        }))
        .map_err(|error| invalid(&error.to_string()))?;
        let settings = raw
            .convert(&input.environment)
            .map_err(|error| invalid(&error.to_string()))?;
        let hide = should_hide_credentials_hint(&settings);
        let form = LoginForm::from_settings(&settings);
        let html = form.html(LoginPageOptions {
            show_deprecation_banner: input.show_deprecation_banner,
            hide_default_credentials_hint: hide,
        });
        facts(&form, &html, hide)
    }
}

fn facts(form: &LoginForm, html: &str, hide: bool) -> HelperResult {
    let (hint, texts) = hint_facts(html)?;
    Ok(json!({
        "hide_default_credentials_hint": hide,
        "form_action": form.action.clone(),
        "new_login_url": form.new_login_url.clone(),
        "form_tag": element(html, "<form ", ">")?,
        "banner": banner_facts(html)?,
        "hint": hint,
        "fields": json!({
            "username_label": element(html, "<label for=\"username\">", "</label>")?,
            "username": element(html, "<input type=\"text\"", ">")?,
            "password_label": element(html, "<label for=\"password\">", "</label>")?,
            "password": element(html, "<input type=\"password\"", ">")?,
            "show_password": element(html, "<input type=\"checkbox\"", ">")?,
            "show_password_label": element(html, "<label for=\"show-password\">", "</label>")?,
            "submit": element(html, "<input type=\"submit\"", ">")?,
        }),
        "toggle_script": lines(between(html, "<script>", "</script>")?),
        "styles": lines(between(html, "<style>", "</style>")?),
        "texts": texts,
    }))
}

fn banner_facts(html: &str) -> HelperResult {
    if html.contains("class=\"deprecation-banner\"") {
        let block = between(html, "<div class=\"deprecation-banner\">", "</div>")?;
        return Ok(json!({
            "link": between(block, "<a href=\"", "\">")?,
            "inner": normalized(block),
        }));
    }
    Ok(Value::Null)
}

fn hint_facts(html: &str) -> Result<(Value, Value), Box<HelperError>> {
    let present = html.contains("class=\"info-box\"");
    let documentation = if present {
        Value::String(element(html, DOCS_LINK, ">")?.to_owned())
    } else {
        Value::Null
    };
    let default_credentials = if present {
        Value::String(normalized(element(html, "<p>By default,", "</p>")?))
    } else {
        Value::Null
    };
    let sso = if present {
        Value::String(normalized(element(
            html,
            "<p>Need to set UI credentials or SSO?",
            "</p>",
        )?))
    } else {
        Value::Null
    };
    let hint = json!({
        "present": present,
        "admin_code": present && html.contains("<code>admin</code>"),
        "master_key_code": present && html.contains("<code>MASTER_KEY</code>"),
        "documentation": documentation,
    });
    let texts = json!({
        "title": normalized(between(html, "<title>", "</title>")?),
        "logo": normalized(between(html, "<div class=\"logo\">", "</div>")?),
        "subtitle": normalized(between(html, "<p class=\"subtitle\">", "</p>")?),
        "default_credentials": default_credentials,
        "sso": sso,
    });
    Ok((hint, texts))
}

fn between<'html>(html: &'html str, opening: &str, closing: &str) -> Markup<'html> {
    html.split_once(opening)
        .and_then(|(_, rest)| rest.split_once(closing))
        .map(|(inner, _)| inner)
        .ok_or_else(|| invalid(MISSING))
}

fn element<'html>(html: &'html str, opening: &str, closing: &str) -> Markup<'html> {
    let start = html.find(opening).ok_or_else(|| invalid(MISSING))?;
    let tail = html.get(start..).ok_or_else(|| invalid(MISSING))?;
    let at = tail.find(closing).ok_or_else(|| invalid(MISSING))?;
    let end = at
        .checked_add(closing.len())
        .ok_or_else(|| invalid(MISSING))?;
    tail.get(..end).ok_or_else(|| invalid(MISSING))
}

fn normalized(block: &str) -> String {
    block
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn lines(block: &str) -> Vec<&str> {
    block
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect()
}

fn invalid(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: "LoginForm".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
