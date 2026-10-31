use crate::arguments::json::Object;

mod budget;
pub mod flatten;
mod regex_walk;

use regex_walk::drop_regexes;

#[must_use]
pub fn drop_non_python_regex_patterns(schema: &Object) -> Object {
    let mut result = schema.clone();
    drop_regexes(&mut result, 0);
    result
}

pub(super) fn flatten_and_drop_schema(schema: &Object) -> Object {
    let filtered = drop_non_python_regex_patterns(schema);
    flatten::flatten_top_level(&filtered)
}
