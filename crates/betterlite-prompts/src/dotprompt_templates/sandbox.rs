use std::borrow::Cow;
use std::path::Path;

use minijinja::value::Value;
use minijinja::{AutoEscape, Environment, Error, Output, State, escape_formatter};
use minijinja_contrib::pycompat::unknown_method_callback;

// LiteLLM's ImmutableSandboxedEnvironment has no such cap; this guards against a template-authored infinite loop.
const FUEL_LIMIT: u64 = 1_000_000;

// minijinja's own Html mode also escapes `/` and spells the quote entities differently; markupsafe does neither.
const MARKUPSAFE: &str = "markupsafe";

#[must_use]
pub(super) fn build<'source>() -> Environment<'source> {
    let mut env = Environment::new();
    env.set_fuel(Some(FUEL_LIMIT));
    env.set_auto_escape_callback(auto_escape_for_name);
    env.set_formatter(format_value);
    env.set_unknown_method_callback(unknown_method_callback);
    env
}

fn auto_escape_for_name(name: &str) -> AutoEscape {
    let extension = Path::new(name).extension().and_then(|ext| ext.to_str());
    match extension {
        Some(ext)
            if ext.eq_ignore_ascii_case("html")
                || ext.eq_ignore_ascii_case("htm")
                || ext.eq_ignore_ascii_case("xml") =>
        {
            AutoEscape::Custom(Cow::Borrowed(MARKUPSAFE))
        }
        // render_str always names its template "<string>"; this is the only name this crate ever renders.
        None if name == "<string>" => AutoEscape::Custom(Cow::Borrowed(MARKUPSAFE)),
        _ => AutoEscape::None,
    }
}

fn format_value(
    out: &mut Output<'_>,
    state: &mut State<'_, '_>,
    value: &Value,
) -> Result<(), Error> {
    match state.auto_escape() {
        AutoEscape::Custom(name) if name.as_ref() == MARKUPSAFE => write_markupsafe(out, value),
        AutoEscape::None | AutoEscape::Html | AutoEscape::Custom(_) | _ => {
            escape_formatter(out, state, value)
        }
    }
}

fn write_markupsafe(out: &mut Output<'_>, value: &Value) -> Result<(), Error> {
    if value.is_safe() {
        return write!(out, "{value}").map_err(Error::from);
    }
    let text = value.to_string();
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
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
