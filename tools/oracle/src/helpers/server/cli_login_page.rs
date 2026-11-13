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
    Verification(Verification),
    Consent(Consent),
    Completion,
}

#[derive(Deserialize)]
pub(crate) struct Verification {
    verify_url: String,
    browser_complete_token: String,
    prefill_user_code: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct Consent {
    client_origin: String,
    user_id: String,
    teams: Vec<Team>,
    flow_handle: String,
    complete_url: String,
}

#[derive(Deserialize)]
pub(crate) struct Team {
    id: String,
    label: String,
}

// the branded title, logo and banner stay out of this table so only the page's contract is compared
#[derive(Clone, Copy)]
enum Field {
    Null,
    Text(&'static str, &'static str),
    Block(&'static str, &'static str),
    Tag(&'static str),
    Lines(&'static str, &'static str),
    OptionalText(&'static str, &'static str),
    OptionalBlock(&'static str, &'static str),
}

const VERIFICATION_FIELDS: &[(&str, Field)] = &[
    ("referrer", Field::Null),
    ("script", Field::Null),
    ("heading", Field::Text("<h1>", "</h1>")),
    ("instructions", Field::Text("<p>", "</p>")),
    ("form", Field::Block("<form ", ">")),
    ("token_input", Field::Tag("name=\"browser_complete_token\"")),
    (
        "code_label",
        Field::Block("<label for=\"user_code\">", "</label>"),
    ),
    ("code_input", Field::Tag("id=\"user_code\"")),
    ("submit", Field::Block("<button type=\"submit\">", "</button>")),
];

const CONSENT_FIELDS: &[(&str, Field)] = &[
    ("referrer", Field::Tag("name=\"referrer\"")),
    ("script", Field::Null),
    ("form", Field::Block("<form ", ">")),
    ("flow_input", Field::Tag("name=\"flow\"")),
    ("origin", Field::Text("<code>", "</code>")),
    ("user", Field::Text("<strong>", "</strong>")),
    (
        "team_attribution",
        Field::OptionalText("<p>Requests are attributed to team ", "</p>"),
    ),
    (
        "team_select",
        Field::OptionalBlock("<select id=\"team_id\"", "</select>"),
    ),
    (
        "deny_button",
        Field::Block(
            "<button type=\"submit\" name=\"decision\" value=\"deny\"",
            "</button>",
        ),
    ),
    (
        "approve_button",
        Field::Block(
            "<button type=\"submit\" name=\"decision\" value=\"approve\"",
            "</button>",
        ),
    ),
];

const COMPLETION_FIELDS: &[(&str, Field)] = &[
    ("referrer", Field::Null),
    ("script", Field::Lines("<script>", "</script>")),
    ("heading", Field::Text("<h1>", "</h1>")),
    ("subtitle", Field::Block("<p class=\"subtitle\">", "</p>")),
    (
        "success_badge",
        Field::Block("<div class=\"success-header\">", "</div>"),
    ),
    (
        "instructions_badge",
        Field::Block("<div class=\"instructions-header\">", "</div>"),
    ),
    (
        "return_instruction",
        Field::Block("<p>Return to your terminal", "</p>"),
    ),
    ("status", Field::Block("<div class=\"status\">", "</div>")),
];

type Markup<'render> = Result<&'render str, Box<HelperError>>;
type Entries = Result<Vec<(&'static str, Value)>, Box<HelperError>>;

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
        Render::Verification(request) => {
            let html = render_verification_page(
                &request.verify_url,
                &request.browser_complete_token,
                request.prefill_user_code.as_deref(),
            );
            Ok(object(field_entries(
                &html,
                "verification",
                VERIFICATION_FIELDS,
            )?))
        }
        Render::Consent(request) => consent_facts(environment, request),
        Render::Completion => Ok(object(field_entries(
            SUCCESS_PAGE,
            "completion",
            COMPLETION_FIELDS,
        )?)),
    }
}

fn consent_facts(environment: &BTreeMap<String, String>, request: &Consent) -> HelperResult {
    let settings = Raw::default()
        .convert(environment)
        .map_err(|error| classified("ConfigError", &error.to_string()))?;
    let pairs: Vec<(String, String)> = request
        .teams
        .iter()
        .map(|team| (team.id.clone(), team.label.clone()))
        .collect();
    let html = render_consent_page(ConsentPage {
        client_origin: request.client_origin.as_str(),
        user_id: request.user_id.as_str(),
        teams: &pairs,
        flow_handle: request.flow_handle.as_str(),
        complete_url: request.complete_url.as_str(),
        cli_expiration_hours: settings.cli_expiration_hours,
    });
    Ok(object(consent_entries(&html)?))
}

fn consent_entries(html: &str) -> Entries {
    let lifetime = between(html, "expires within ", " hours")?
        .trim()
        .parse::<i64>()
        .map_err(|error| classified("ValueError", &error.to_string()))?;
    let logout = html
        .contains("<code>lite logout</code>")
        .then_some("lite logout");
    let team_hidden = if html.contains("<input type=\"hidden\" name=\"team_id\"") {
        tag_text(html, "name=\"team_id\"")?
    } else {
        Value::Null
    };
    let mut entries = field_entries(html, "consent", CONSENT_FIELDS)?;
    entries.push(("lifetime_hours", json!(lifetime)));
    entries.push(("logout_command", json!(logout)));
    entries.push(("team_hidden", team_hidden));
    Ok(entries)
}

fn field_entries(html: &str, page: &'static str, fields: &[(&'static str, Field)]) -> Entries {
    let mut entries: Vec<(&'static str, Value)> = vec![("page", json!(page))];
    for (key, field) in fields {
        entries.push((*key, field.value(html)?));
    }
    Ok(entries)
}

impl Field {
    fn value(self, html: &str) -> HelperResult {
        match self {
            Self::Null => Ok(Value::Null),
            Self::Text(opening, closing) => text(html, opening, closing),
            Self::Block(opening, closing) => block_text(html, opening, closing),
            Self::Tag(needle) => tag_text(html, needle),
            Self::Lines(opening, closing) => Ok(json!(lines(between(html, opening, closing)?))),
            Self::OptionalText(opening, closing) => optional_text(html, opening, closing),
            Self::OptionalBlock(opening, closing) => optional_block(html, opening, closing),
        }
    }
}

fn text(html: &str, opening: &str, closing: &str) -> HelperResult {
    between(html, opening, closing).map(|raw| json!(normalized(raw)))
}

fn block_text(html: &str, opening: &str, closing: &str) -> HelperResult {
    element(html, opening, closing).map(|raw| json!(normalized(raw)))
}

fn tag_text(html: &str, needle: &str) -> HelperResult {
    tag_with(html, needle).map(|raw| json!(normalized(raw)))
}

fn optional_text(html: &str, opening: &str, closing: &str) -> HelperResult {
    if !html.contains(opening) {
        return Ok(Value::Null);
    }
    text(html, opening, closing)
}

fn optional_block(html: &str, opening: &str, closing: &str) -> HelperResult {
    if !html.contains(opening) {
        return Ok(Value::Null);
    }
    block_text(html, opening, closing)
}

fn object(entries: Vec<(&'static str, Value)>) -> Value {
    Value::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn between<'render>(html: &'render str, opening: &str, closing: &str) -> Markup<'render> {
    html.split_once(opening)
        .and_then(|(_head, rest)| rest.split_once(closing))
        .map(|(inner, _tail)| inner)
        .ok_or_else(|| classified("ValueError", MISSING))
}

fn element<'render>(html: &'render str, opening: &str, closing: &str) -> Markup<'render> {
    let start = found(html.find(opening))?;
    let tail = found(html.get(start..))?;
    let offset = found(tail.find(closing))?;
    let end = found(offset.checked_add(closing.len()))?;
    found(tail.get(..end))
}

fn tag_with<'render>(html: &'render str, needle: &str) -> Markup<'render> {
    let at = found(html.find(needle))?;
    let start = found(html.get(..at).and_then(|head| head.rfind('<')))?;
    let close = found(html.get(at..).and_then(|tail| tail.find('>')))?;
    let end = found(close.checked_add(at).and_then(|index| index.checked_add(1)))?;
    found(html.get(start..end))
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

fn found<T>(value: Option<T>) -> Result<T, Box<HelperError>> {
    value.ok_or_else(|| classified("ValueError", MISSING))
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
