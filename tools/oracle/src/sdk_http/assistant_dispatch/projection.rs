use betterlite_error::envelope::WireField;

use super::super::result::{Failure, Nullable};
use super::model::Step;

// A successful wrapper call carries the ids it returned and, for a polled run, its status.
pub(super) const fn record(ids: Vec<String>, status: Option<String>) -> Step {
    Step {
        ids,
        status,
        events: None,
        failure: None,
    }
}

// The factory's own exceptions carry no status, type, param or code.
pub(super) fn named(class: &str, message: String) -> Failure {
    Failure {
        class: class.to_owned(),
        status: 0,
        kind: None,
        param: None,
        code: None,
        message,
    }
}

// None: key absent, Some(None): JSON null, Some(Some(text)): text
pub(super) fn nullable(field: WireField) -> Nullable {
    match field {
        WireField::Missing(_) => None,
        WireField::Null => Some(None),
        WireField::Text(text) => Some(Some(text)),
    }
}
