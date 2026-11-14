use betterlite_openai::openai_base_config::Object;
use serde_json::{from_value, to_value};

use super::error::RecordsError;
use super::{SONIOX_HANDLER_ONLY_PARAMS, SONIOX_PASSTHROUGH_PARAMS};

pub const LANGUAGE_HINTS: &str = "language_hints";
const LANGUAGE: &str = "language";
const RESPONSE_FORMAT: &str = "response_format";

pub fn map_openai_params(non_default: &Object, optional: &mut Object) -> Result<(), RecordsError> {
    merge_language_hints(non_default, optional)?;
    copy_response_format(non_default, optional);
    copy_passthrough(non_default, optional);
    Ok(())
}

fn merge_language_hints(non_default: &Object, optional: &mut Object) -> Result<(), RecordsError> {
    let Some(value) = non_default.get(LANGUAGE) else {
        return Ok(());
    };
    let language: Option<String> =
        from_value(value.clone()).map_err(|source| RecordsError::Value {
            field: LANGUAGE,
            source,
        })?;
    let Some(language) = language.filter(|language| !language.is_empty()) else {
        return Ok(());
    };
    let existing = language_hints(optional)?;
    if existing.is_empty() {
        return write_hints(optional, &[language]);
    }
    if existing.contains(&language) {
        return Ok(());
    }
    let mut merged = Vec::new();
    merged.push(language);
    merged.extend(existing);
    write_hints(optional, &merged)
}

fn language_hints(optional: &Object) -> Result<Vec<String>, RecordsError> {
    let Some(value) = optional.get(LANGUAGE_HINTS) else {
        return Ok(Vec::new());
    };
    let hints: Option<Vec<String>> =
        from_value(value.clone()).map_err(|source| RecordsError::Value {
            field: LANGUAGE_HINTS,
            source,
        })?;
    Ok(hints.unwrap_or_default())
}

fn write_hints(optional: &mut Object, hints: &[String]) -> Result<(), RecordsError> {
    let encoded = to_value(hints).map_err(|source| RecordsError::Value {
        field: LANGUAGE_HINTS,
        source,
    })?;
    optional.insert(LANGUAGE_HINTS.to_owned(), encoded);
    Ok(())
}

fn copy_response_format(non_default: &Object, optional: &mut Object) {
    if let Some(value) = non_default.get(RESPONSE_FORMAT) {
        optional.insert(RESPONSE_FORMAT.to_owned(), value.clone());
    }
}

fn copy_passthrough(non_default: &Object, optional: &mut Object) {
    let names = SONIOX_PASSTHROUGH_PARAMS
        .into_iter()
        .chain(SONIOX_HANDLER_ONLY_PARAMS);
    for name in names {
        if let Some(value) = non_default.get(name).filter(|value| !value.is_null()) {
            optional.insert(name.to_owned(), value.clone());
        }
    }
}
