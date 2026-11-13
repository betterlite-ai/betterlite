use std::collections::BTreeMap;

use betterlite_config::sections::Section as _;
use betterlite_config::sections::session_token_codec::Raw;
use betterlite_server::cli_login_pages::{
    ConsentPage, SUCCESS_PAGE, render_consent_page, render_verification_page,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
    environment: BTreeMap<String, String>,
    renders: Vec<Render>,
}

#[derive(Deserialize)]
#[serde(tag = "page", rename_all = "snake_case")]
pub(crate) enum Render {
    Verification {
        verify_url: String,
        browser_complete_token: String,
        prefill_user_code: Option<String>,
    },
    Consent {
        client_origin: String,
        user_id: String,
        teams: Vec<Team>,
        flow_handle: String,
        complete_url: String,
    },
    Completion,
}

#[derive(Deserialize)]
pub(crate) struct Team {
    id: String,
    label: String,
}

type Markup<'render> = Result<&'render str, Box<HelperError>>;

const MISSING: &str = "the rendered page is missing a boundary";

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut facts = Vec::with_capacity(input.renders.len());
        for render in &input.renders {
            facts.push(facts_for(render, &input.environment)?);
        }
        Ok(Value::Array(facts))
    }
}

fn facts_for(render: &Render, environment: &BTreeMap<String, String>) -> HelperResult {
    match render {
        Render::Verification {
            verify_url,
            browser_complete_token,
            prefill_user_code,
        } => {
            let html = render_verification_page(
                verify_url,
                browser_complete_token,
                prefill_user_code.as_deref(),
            );
            verification_facts(&html)
        }
        Render::Consent {
            client_origin,
            user_id,
            teams,
            flow_handle,
            complete_url,
        } => {
            let settings = Raw::default()
                .convert(environment)
                .map_err(|error| classified("ConfigError", &error.to_string()))?;
            let pairs: Vec<(String, String)> = teams
                .iter()
                .map(|team| (team.id.clone(), team.label.clone()))
                .collect();
            let html = render_consent_page(ConsentPage {
                client_origin: client_origin.as_str(),
                user_id: user_id.as_str(),
                teams: &pairs,
                flow_handle: flow_handle.as_str(),
                complete_url: complete_url.as_str(),
                cli_expiration_hours: settings.cli_expiration_hours,
            });
            consent_facts(&html)
        }
        Render::Completion => completion_facts(SUCCESS_PAGE),
    }
}

fn verification_facts(html: &str) -> HelperResult {
    Ok(json!({
        "page": "verification",
        "referrer": Value::Null,
        "script": Value::Null,
        "heading": normalized(between(html, "<h1>", "</h1>")?),
        "instructions": normalized(between(html, "<p>", "</p>")?),
        "form": normalized(element(html, "<form ", ">")?),
        "token_input": normalized(tag_with(html, "name=\"browser_complete_token\"")?),
        "code_label": normalized(element(html, "<label for=\"user_code\">", "</label>")?),
        "code_input": normalized(tag_with(html, "id=\"user_code\"")?),
        "submit": normalized(element(html, "<button type=\"submit\">", "</button>")?),
    }))
}

fn consent_facts(html: &str) -> HelperResult {
    let lifetime = between(html, "expires within ", " hours")?
        .trim()
        .parse::<i64>()
        .map_err(|error| classified("ValueError", &error.to_string()))?;
    let team_hidden = if html.contains("<input type=\"hidden\" name=\"team_id\"") {
        Some(normalized(tag_with(html, "name=\"team_id\"")?))
    } else {
        None
    };
    let logout = html
        .contains("<code>lite logout</code>")
        .then_some("lite logout");
    Ok(json!({
        "page": "consent",
        "referrer": normalized(tag_with(html, "name=\"referrer\"")?),
        "script": Value::Null,
        "form": normalized(element(html, "<form ", ">")?),
        "flow_input": normalized(tag_with(html, "name=\"flow\"")?),
        "origin": normalized(between(html, "<code>", "</code>")?),
        "user": normalized(between(html, "<strong>", "</strong>")?),
        "lifetime_hours": lifetime,
        "logout_command": logout,
        "team_hidden": team_hidden,
        "team_attribution": optional_between(html, "<p>Requests are attributed to team ", "</p>")?,
        "team_select": optional_element(html, "<select id=\"team_id\"", "</select>")?,
        "deny_button": normalized(element(html, "<button type=\"submit\" name=\"decision\" value=\"deny\"", "</button>")?),
        "approve_button": normalized(element(html, "<button type=\"submit\" name=\"decision\" value=\"approve\"", "</button>")?),
    }))
}

fn completion_facts(html: &str) -> HelperResult {
    Ok(json!({
        "page": "completion",
        "referrer": Value::Null,
        "script": lines(between(html, "<script>", "</script>")?),
        "heading": normalized(between(html, "<h1>", "</h1>")?),
        "subtitle": normalized(element(html, "<p class=\"subtitle\">", "</p>")?),
        "success_badge": normalized(element(html, "<div class=\"success-header\">", "</div>")?),
        "instructions_badge": normalized(element(html, "<div class=\"instructions-header\">", "</div>")?),
        "return_instruction": normalized(element(html, "<p>Return to your terminal", "</p>")?),
        "status": normalized(element(html, "<div class=\"status\">", "</div>")?),
    }))
}

fn between<'render>(html: &'render str, opening: &str, closing: &str) -> Markup<'render> {
    html.split_once(opening)
        .and_then(|(_head, rest)| rest.split_once(closing))
        .map(|(inner, _tail)| inner)
        .ok_or_else(|| classified("ValueError", MISSING))
}

fn element<'render>(html: &'render str, opening: &str, closing: &str) -> Markup<'render> {
    let start = html
        .find(opening)
        .ok_or_else(|| classified("ValueError", MISSING))?;
    let tail = html
        .get(start..)
        .ok_or_else(|| classified("ValueError", MISSING))?;
    let at = tail
        .find(closing)
        .ok_or_else(|| classified("ValueError", MISSING))?;
    let end = at
        .checked_add(closing.len())
        .ok_or_else(|| classified("ValueError", MISSING))?;
    tail.get(..end)
        .ok_or_else(|| classified("ValueError", MISSING))
}

fn tag_with<'render>(html: &'render str, needle: &str) -> Markup<'render> {
    let at = html
        .find(needle)
        .ok_or_else(|| classified("ValueError", MISSING))?;
    let head = html
        .get(..at)
        .ok_or_else(|| classified("ValueError", MISSING))?;
    let start = head
        .rfind('<')
        .ok_or_else(|| classified("ValueError", MISSING))?;
    let tail = html
        .get(at..)
        .ok_or_else(|| classified("ValueError", MISSING))?;
    let close = tail
        .find('>')
        .ok_or_else(|| classified("ValueError", MISSING))?;
    let end = at
        .checked_add(close)
        .and_then(|index| index.checked_add(1))
        .ok_or_else(|| classified("ValueError", MISSING))?;
    html.get(start..end)
        .ok_or_else(|| classified("ValueError", MISSING))
}

fn optional_between(
    html: &str,
    opening: &str,
    closing: &str,
) -> Result<Option<String>, Box<HelperError>> {
    if !html.contains(opening) {
        return Ok(None);
    }
    between(html, opening, closing).map(|inner| Some(normalized(inner)))
}

fn optional_element(
    html: &str,
    opening: &str,
    closing: &str,
) -> Result<Option<String>, Box<HelperError>> {
    if !html.contains(opening) {
        return Ok(None);
    }
    element(html, opening, closing).map(|raw| Some(normalized(raw)))
}

fn normalized(block: &str) -> String {
    block
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn lines(block: &str) -> Vec<String> {
    block
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

fn classified(class: &str, message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
