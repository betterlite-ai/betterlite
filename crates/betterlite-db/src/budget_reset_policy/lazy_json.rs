use serde::Serialize;
use serde_json::to_string_pretty;
use std::fmt;

// _LazyJson: a logging argument that serializes only when the formatter writes the record, so a dropped log level costs no dump.
pub struct LazyJson<'value, Value: ?Sized>(pub &'value Value);

impl<Value> fmt::Debug for LazyJson<'_, Value>
where
    Value: Serialize + ?Sized,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match to_string_pretty(self.0) {
            Ok(text) => formatter.write_str(&text),
            Err(_) => Err(fmt::Error),
        }
    }
}

mod tests;
