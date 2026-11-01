use std::path::Path;

use minijinja::{AutoEscape, Environment};

// LiteLLM's ImmutableSandboxedEnvironment has no such cap; this guards against a template-authored infinite loop.
const FUEL_LIMIT: u64 = 1_000_000;

#[must_use]
pub(super) fn build<'source>() -> Environment<'source> {
    let mut env = Environment::new();
    env.set_fuel(Some(FUEL_LIMIT));
    env.set_auto_escape_callback(auto_escape_for_name);
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
            AutoEscape::Html
        }
        _ => AutoEscape::None,
    }
}

mod tests;
