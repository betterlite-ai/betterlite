use std::borrow::Cow;
use std::fmt::Write as _;

use betterlite_models::decisions::Json;
use betterlite_models::prompt_contracts::PromptParams;
use indexmap::IndexMap;
use minijinja::value::Value;
use minijinja::{AutoEscape, Environment, Error, Output, State, escape_formatter};
use minijinja_contrib::pycompat::unknown_method_callback;
use thiserror::Error;

use crate::arize::phoenix::records::{ContentPart, PromptTemplate};

// LiteLLM's ImmutableSandboxedEnvironment has no such cap; this guards against a template-authored infinite loop.
const FUEL_LIMIT: u64 = 1_000_000;

// minijinja's own Html mode also escapes `/` and spells the quote entities differently; markupsafe does neither.
const MARKUPSAFE: &str = "markupsafe";

const ENTITIES: [(char, &str); 5] = [
    ('&', "&amp;"),
    ('<', "&lt;"),
    ('>', "&gt;"),
    ('\'', "&#39;"),
    ('"', "&#34;"),
];

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("{source}")]
    Template {
        #[source]
        source: Error,
    },
    #[error("sequence item {index}: expected str instance, dict found")]
    NonTextContent { index: usize },
}

// minijinja has no attribute access to host objects, so templates from a Phoenix workspace cannot reach `__class__` chains.
#[must_use]
pub fn sandbox() -> Environment<'static> {
    let mut environment = Environment::new();
    environment.set_fuel(Some(FUEL_LIMIT));
    environment.set_auto_escape_callback(|_| AutoEscape::Custom(Cow::Borrowed(MARKUPSAFE)));
    environment.set_formatter(format_value);
    environment.set_unknown_method_callback(unknown_method_callback);
    environment
}

fn format_value(
    out: &mut Output<'_>,
    state: &mut State<'_, '_>,
    value: &Value,
) -> Result<(), Error> {
    let escapes =
        matches!(state.auto_escape(), AutoEscape::Custom(name) if name.as_ref() == MARKUPSAFE);
    if !escapes || value.is_safe() {
        return escape_formatter(out, state, value);
    }
    for character in value.to_string().chars() {
        match ENTITIES.iter().find(|(raw, _)| *raw == character) {
            Some((_, entity)) => out.write_str(entity)?,
            None => out.write_char(character)?,
        }
    }
    Ok(())
}

// serde_json's Number serializes as a private struct under the workspace's arbitrary_precision, so the Serde adapter cannot be used.
fn variable_value(json: &Json) -> Value {
    match json {
        Json::Null => Value::from(()),
        Json::Bool(flag) => Value::from(*flag),
        Json::Number(number) => match (number.as_i64(), number.as_u64(), number.as_f64()) {
            (Some(integer), _, _) => Value::from(integer),
            (None, Some(unsigned), _) => Value::from(unsigned),
            (None, None, Some(fraction)) => Value::from(fraction),
            (None, None, None) => Value::from(number.to_string()),
        },
        Json::Text(text) => Value::from(text.as_str()),
        Json::List(items) => items.iter().map(variable_value).collect(),
        Json::Object(entries) => Value::from_pairs(
            entries
                .iter()
                .map(|(key, entry)| (key.as_str(), variable_value(entry))),
        ),
    }
}

fn render_content(
    environment: &Environment<'_>,
    parts: &[ContentPart],
    context: &Value,
) -> Result<String, RenderError> {
    let mut rendered = Vec::with_capacity(parts.len());
    let mut first_other = None;
    for (index, part) in parts.iter().enumerate() {
        if part.kind == "text" {
            let text = environment
                .render_str(&part.text, context)
                .map_err(|source| RenderError::Template { source })?;
            rendered.push(text);
        } else {
            first_other.get_or_insert(index);
        }
    }
    if let Some(index) = first_other {
        return Err(RenderError::NonTextContent { index });
    }
    Ok(rendered.join(" "))
}

pub fn render_messages(
    environment: &Environment<'_>,
    template: &PromptTemplate,
    variables: Option<&PromptParams>,
) -> Result<Vec<Json>, RenderError> {
    let context = Value::from_pairs(
        variables
            .into_iter()
            .flatten()
            .map(|(name, value)| (name.as_str(), variable_value(value))),
    );
    let mut messages = Vec::with_capacity(template.messages.len());
    for message in &template.messages {
        let body = render_content(environment, &message.content, &context)?;
        messages.push(Json::Object(IndexMap::from([
            ("role".to_owned(), Json::Text(message.role.clone())),
            ("content".to_owned(), Json::Text(body)),
        ])));
    }
    Ok(messages)
}

mod tests;
