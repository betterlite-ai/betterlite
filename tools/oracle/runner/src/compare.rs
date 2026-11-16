use crate::case::Ignore;

pub mod helper;
pub mod value;

use helper::HelperOutput;
use value::{first_difference, remove_pointer};

#[derive(Debug, serde::Serialize)]
pub struct Difference {
    pub message: String,
    pub pointer: String,
}

fn rejected_helper_ignore(pointer: &str) -> Option<Difference> {
    if pointer == "/error/class" || pointer == "/error/message" {
        return Some(Difference {
            message: "compare.ignore names a field helper comparisons skip".to_owned(),
            pointer: pointer.to_owned(),
        });
    }
    None
}

fn strip_helper_only_fields(
    left: &HelperOutput,
    right: &HelperOutput,
    left_value: &mut serde_json::Value,
    right_value: &mut serde_json::Value,
) {
    remove_pointer(left_value, "/error/class");
    remove_pointer(left_value, "/error/message");
    remove_pointer(right_value, "/error/class");
    remove_pointer(right_value, "/error/message");
    for index in 0..left.warnings.len() {
        remove_pointer(left_value, &format!("/warnings/{index}/message"));
    }
    for index in 0..right.warnings.len() {
        remove_pointer(right_value, &format!("/warnings/{index}/message"));
    }
}

fn unknown_helper_ignore(
    ignored: &Ignore,
    left_value: &serde_json::Value,
    right_value: &serde_json::Value,
) -> Option<Difference> {
    let known = matches!(
        ignored.pointer.as_str(),
        "/ok"
            | "/value"
            | "/error"
            | "/error/status"
            | "/error/type"
            | "/error/param"
            | "/error/code"
            | "/warnings"
            | "/effects"
    );
    let found = left_value.pointer(&ignored.pointer).is_some()
        || right_value.pointer(&ignored.pointer).is_some();
    (!found && !known).then(|| Difference {
        message: "compare.ignore points to no field".to_owned(),
        pointer: ignored.pointer.clone(),
    })
}

#[must_use]
pub fn compare_helper(
    left: &HelperOutput,
    right: &HelperOutput,
    ignores: &[Ignore],
) -> Option<Difference> {
    for ignored in ignores {
        if let Some(difference) = rejected_helper_ignore(&ignored.pointer) {
            return Some(difference);
        }
    }
    let mut left_value = serde_json::to_value(left).ok()?;
    let mut right_value = serde_json::to_value(right).ok()?;
    for ignored in ignores {
        if let Some(difference) = unknown_helper_ignore(ignored, &left_value, &right_value) {
            return Some(difference);
        }
    }
    if let Some(difference) = compare_effect_secrets(left, right, ignores) {
        return Some(difference);
    }
    strip_helper_only_fields(left, right, &mut left_value, &mut right_value);
    for ignored in ignores {
        remove_pointer(&mut left_value, &ignored.pointer);
        remove_pointer(&mut right_value, &ignored.pointer);
    }
    first_difference(&left_value, &right_value, "").map(|pointer| Difference {
        pointer,
        message: "helper results differ".to_owned(),
    })
}

fn compare_effect_secrets(
    left: &HelperOutput,
    right: &HelperOutput,
    ignores: &[Ignore],
) -> Option<Difference> {
    if left.effects.len() != right.effects.len() {
        return None;
    }
    left.effects
        .iter()
        .zip(&right.effects)
        .enumerate()
        .find_map(|(index, (left_effect, right_effect))| {
            if effect_secret_is_ignored(ignores, index)
                || left_effect.fields.same_secret(&right_effect.fields)
            {
                None
            } else {
                Some(Difference {
                    message: "helper results differ".to_owned(),
                    pointer: format!("/effects/{index}/fields/credential"),
                })
            }
        })
}

fn effect_secret_is_ignored(ignores: &[Ignore], index: usize) -> bool {
    let prefix = format!("/effects/{index}/fields/credential");
    ignores.iter().any(|ignore| {
        prefix == ignore.pointer || prefix.starts_with(&format!("{}/", ignore.pointer))
    })
}
