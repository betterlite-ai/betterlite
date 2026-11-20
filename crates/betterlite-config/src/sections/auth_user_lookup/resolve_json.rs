use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;

use super::{MAX_NESTED_DEPTH, RawValue, invalid};
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;

// proxy_server.py resolves os.environ/ in mappings and mapping list items; strings under a plain list stay text
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    Mapping,
    Nested,
}

#[derive(Clone, Copy)]
struct Resolve<'lookup> {
    lookup: &'lookup dyn EnvironmentLookup,
    depth: usize,
    place: Place,
}

impl Resolve<'_> {
    #[must_use]
    const fn child(self, place: Place) -> Self {
        let depth = self.depth.saturating_add(1);
        Self {
            lookup: self.lookup,
            depth,
            place,
        }
    }
}

pub(super) fn resolved(
    value: RawValue,
    path: &str,
    lookup: &dyn EnvironmentLookup,
) -> Result<JsonData, ConfigError> {
    let root = Resolve {
        lookup,
        depth: 0,
        place: Place::Mapping,
    };
    to_json(value, path, root)
}

fn to_json(value: RawValue, path: &str, resolve: Resolve<'_>) -> Result<JsonData, ConfigError> {
    if resolve.depth > MAX_NESTED_DEPTH {
        return Err(invalid(path, false));
    }
    match value {
        RawValue::Null => Ok(JsonData::Null),
        RawValue::Boolean(flag) => Ok(JsonData::Bool(flag)),
        RawValue::Integer(number) => Ok(JsonData::Number(serde_json::Number::from(number))),
        RawValue::Float(number) => serde_json::Number::from_f64(number)
            .map(JsonData::Number)
            .ok_or_else(|| invalid(path, false)),
        RawValue::Text(text) if resolve.place == Place::Mapping => {
            resolve_string(text, path, resolve.lookup).map(JsonData::String)
        }
        RawValue::Text(text) => Ok(JsonData::String(text)),
        RawValue::Object(fields) => {
            let mut resolved = BTreeMap::new();
            for (key, value) in fields {
                let child_path = format!("{path}.{key}");
                let kept = to_json(value, &child_path, resolve.child(resolve.place))?;
                resolved.insert(key, kept);
            }
            Ok(JsonData::Object(resolved))
        }
        RawValue::Array(items) => {
            let mut resolved = Vec::with_capacity(items.len());
            for item in items {
                let place =
                    if resolve.place == Place::Mapping && matches!(item, RawValue::Object(_)) {
                        Place::Mapping
                    } else {
                        Place::Nested
                    };
                resolved.push(to_json(item, path, resolve.child(place))?);
            }
            Ok(JsonData::Array(resolved))
        }
    }
}
