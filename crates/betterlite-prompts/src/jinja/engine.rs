use chrono::{DateTime, Utc};
use minijinja::{Environment, Error, ErrorKind};
use std::fmt::Write as _;

const FUEL_LIMIT: u64 = 1_000_000;

fn raise_exception(message: String) -> String {
    format!("Error message - {message}")
}

fn strftime_now(now: DateTime<Utc>, format: String) -> Result<String, Error> {
    let mut rendered = String::new();
    write!(rendered, "{}", now.format(&format))
        .map_err(|_| Error::new(ErrorKind::InvalidOperation, "strftime format string cannot be applied"))?;
    Ok(rendered)
}

pub(super) fn build<'source>(now: DateTime<Utc>) -> Environment<'source> {
    let mut env = Environment::new();
    env.set_debug(true);
    env.set_fuel(Some(FUEL_LIMIT));
    env.add_function("raise_exception", raise_exception);
    env.add_function("strftime_now", move |format: String| strftime_now(now, format));
    env.set_unknown_method_callback(minijinja_contrib::pycompat::unknown_method_callback);
    env
}

mod tests;
