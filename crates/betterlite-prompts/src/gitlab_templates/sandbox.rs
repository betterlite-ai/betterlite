use std::borrow::Cow;

use minijinja::value::Value;
use minijinja::{AutoEscape, Environment, Error, Output, State, escape_formatter};
use minijinja_contrib::pycompat::unknown_method_callback;
use thiserror::Error;

use crate::arguments::json::{Json, Object};

// LiteLLM's ImmutableSandboxedEnvironment has no such cap; this guards against a template-authored infinite loop.
const FUEL_LIMIT: u64 = 1_000_000;

// minijinja's own Html mode also escapes `/` and spells the quote entities differently; markupsafe does neither.
const MARKUPSAFE: &str = "markupsafe";

#[derive(Debug, Error)]
#[error("Error rendering template '{template_id}': {source}")]
pub struct SandboxError {
    template_id: String,
    #[source]
    source: Error,
}

// dotprompt's sandbox module is private, so this renderer builds its own environment.
pub fn render_sandboxed(
    template_id: &str,
    content: &str,
    variables: &Object,
) -> Result<String, SandboxError> {
    let mut env = Environment::new();
    env.set_fuel(Some(FUEL_LIMIT));
    env.set_auto_escape_callback(|_| AutoEscape::Custom(Cow::Borrowed(MARKUPSAFE)));
    env.set_formatter(format_value);
    env.set_unknown_method_callback(unknown_method_callback);
    let scope = Value::from_pairs(
        variables
            .iter()
            .map(|(name, value)| (name.clone(), to_value(value))),
    );
    env.render_str(content, scope)
        .map_err(|source| SandboxError {
            template_id: template_id.to_owned(),
            source,
        })
}

pub(crate) fn to_value(json: &Json) -> Value {
    match json {
        Json::Null => Value::from(()),
        Json::Bool(flag) => Value::from(*flag),
        Json::Number(number) => number
            .as_i64()
            .map(Value::from)
            .or_else(|| number.as_u64().map(Value::from))
            .or_else(|| number.as_f64().map(Value::from))
            .unwrap_or_else(|| Value::from(number.to_string())),
        Json::Text(text) => Value::from(text.as_str()),
        Json::List(items) => items.iter().map(to_value).collect(),
        Json::Object(fields) => Value::from_pairs(
            fields
                .iter()
                .map(|(name, value)| (name.clone(), to_value(value))),
        ),
    }
}

fn format_value(
    out: &mut Output<'_>,
    state: &mut State<'_, '_>,
    value: &Value,
) -> Result<(), Error> {
    if !matches!(state.auto_escape(), AutoEscape::Custom(name) if name == MARKUPSAFE) {
        return escape_formatter(out, state, value);
    }
    if value.is_safe() {
        return write!(out, "{value}").map_err(Error::from);
    }
    let mut escaped = String::new();
    for ch in value.to_string().chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '\'' => escaped.push_str("&#39;"),
            '"' => escaped.push_str("&#34;"),
            other => escaped.push(other),
        }
    }
    out.write_str(&escaped).map_err(Error::from)
}

mod tests;
