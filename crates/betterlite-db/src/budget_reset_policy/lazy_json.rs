use serde::Serialize;
use serde_json::to_string_pretty;
use std::fmt;

// _LazyJson: a logging argument that serializes only when the formatter writes the record, so a dropped log level costs no dump.
pub struct LazyJson<'value, Value: ?Sized>(pub &'value Value);

impl<Value> fmt::Debug for LazyJson<'_, Value>
where
    Value: Serialize + ?Sized,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        to_string_pretty(self.0).map_or(Err(fmt::Error), |text| f.write_str(&text))
    }
}

mod tests;
