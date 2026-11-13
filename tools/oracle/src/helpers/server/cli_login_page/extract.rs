use serde_json::{Value, json};

use crate::helpers::{HelperError, HelperResult};

pub(super) type Markup<'render> = Result<&'render str, Box<HelperError>>;
pub(super) type Entries = Result<Vec<(&'static str, Value)>, Box<HelperError>>;

const MISSING: &str = "the rendered page is missing a boundary";

// the branded title, logo and banner stay out of this table so only the page's contract is compared
#[derive(Clone, Copy)]
pub(super) enum Field {
    Null,
    Text(&'static str, &'static str),
    Block(&'static str, &'static str),
    Lines(&'static str, &'static str),
    OptionalText(&'static str, &'static str),
    OptionalBlock(&'static str, &'static str),
}

pub(super) const VERIFICATION_FIELDS: &[(&str, Field)] = &[
    ("referrer", Field::Null),
    ("script", Field::Null),
    ("heading", Field::Text("<h1>", "</h1>")),
    ("instructions", Field::Text("<p>", "</p>")),
    ("form", Field::Block("<form ", ">")),
    (
        "token_input",
        Field::Block(
            "<input type=\"hidden\" name=\"browser_complete_token\"",
            ">",
        ),
    ),
    (
        "code_label",
        Field::Block("<label for=\"user_code\">", "</label>"),
    ),
    ("code_input", Field::Block("<input id=\"user_code\"", ">")),
    (
        "submit",
        Field::Block("<button type=\"submit\">", "</button>"),
    ),
];

pub(super) const CONSENT_FIELDS: &[(&str, Field)] = &[
    ("referrer", Field::Block("<meta name=\"referrer\"", ">")),
    ("script", Field::Null),
    ("form", Field::Block("<form ", ">")),
    (
        "flow_input",
        Field::Block("<input type=\"hidden\" name=\"flow\"", ">"),
    ),
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

pub(super) const COMPLETION_FIELDS: &[(&str, Field)] = &[
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

impl Field {
    fn value(self, html: &str) -> HelperResult {
        match self {
            Self::Null => Ok(Value::Null),
            Self::Text(opening, closing) => {
                between(html, opening, closing).map(|raw| json!(normalized(raw)))
            }
            Self::Block(opening, closing) => {
                element(html, opening, closing).map(|raw| json!(normalized(raw)))
            }
            Self::Lines(opening, closing) => Ok(json!(lines(between(html, opening, closing)?))),
            Self::OptionalText(opening, closing) => optional_text(html, opening, closing),
            Self::OptionalBlock(opening, closing) => optional_block(html, opening, closing),
        }
    }
}

pub(super) fn consent_entries(html: &str) -> Entries {
    let lifetime = between(html, "expires within ", " hours")?
        .trim()
        .parse::<i64>()
        .map_err(|error| classified("ValueError", &error.to_string()))?;
    let logout = html
        .contains("<code>lite logout</code>")
        .then_some("lite logout");
    let team_hidden = if html.contains("<input type=\"hidden\" name=\"team_id\"") {
        element(html, "<input type=\"hidden\" name=\"team_id\"", ">")
            .map(|raw| json!(normalized(raw)))?
    } else {
        Value::Null
    };
    let mut entries = field_entries(html, "consent", CONSENT_FIELDS)?;
    entries.push(("lifetime_hours", json!(lifetime)));
    entries.push(("logout_command", json!(logout)));
    entries.push(("team_hidden", team_hidden));
    Ok(entries)
}

pub(super) fn field_entries(
    html: &str,
    page: &'static str,
    fields: &[(&'static str, Field)],
) -> Entries {
    let mut entries: Vec<(&'static str, Value)> = vec![("page", json!(page))];
    for (key, field) in fields {
        entries.push((*key, field.value(html)?));
    }
    Ok(entries)
}

pub(super) fn object(entries: Vec<(&'static str, Value)>) -> Value {
    Value::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

pub(super) fn classified(class: &str, message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

fn optional_text(html: &str, opening: &str, closing: &str) -> HelperResult {
    if !html.contains(opening) {
        return Ok(Value::Null);
    }
    between(html, opening, closing).map(|raw| json!(normalized(raw)))
}

fn optional_block(html: &str, opening: &str, closing: &str) -> HelperResult {
    if !html.contains(opening) {
        return Ok(Value::Null);
    }
    element(html, opening, closing).map(|raw| json!(normalized(raw)))
}

fn between<'render>(html: &'render str, opening: &str, closing: &str) -> Markup<'render> {
    html.split_once(opening)
        .and_then(|(_head, rest)| rest.split_once(closing))
        .map(|(inner, _tail)| inner)
        .ok_or_else(|| classified("ValueError", MISSING))
}

fn element<'render>(html: &'render str, opening: &str, closing: &str) -> Markup<'render> {
    let inner = between(html, opening, closing)?;
    let start = html
        .find(opening)
        .ok_or_else(|| classified("ValueError", MISSING))?;
    let inner_start = start
        .checked_add(opening.len())
        .ok_or_else(|| classified("ValueError", MISSING))?;
    let inner_end = inner_start
        .checked_add(inner.len())
        .ok_or_else(|| classified("ValueError", MISSING))?;
    let end = inner_end
        .checked_add(closing.len())
        .ok_or_else(|| classified("ValueError", MISSING))?;
    html.get(start..end)
        .ok_or_else(|| classified("ValueError", MISSING))
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
