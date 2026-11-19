use betterlite_models::decisions::Json;

pub(super) fn positive_integer(body: &Json, field: &str) -> Option<i64> {
    let Json::Object(fields) = body else {
        return None;
    };
    match fields.get(field) {
        Some(Json::Number(number)) => number.as_i64().filter(|limit| *limit > 0),
        _ => None,
    }
}

mod tests;
