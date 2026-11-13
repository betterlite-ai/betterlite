use betterlite_config::sections::settings::JsonValue;

use super::super::{ENV_KEY, SettingsDocument};
use super::value::{ABSENT, OwnedValue};

pub(super) fn lookup(settings: &SettingsDocument, path: &str) -> OwnedValue {
    match path.rsplit_once('.') {
        None => owned(settings.get(path)),
        Some((_section, key)) => owned(
            settings
                .get(ENV_KEY)
                .and_then(JsonValue::as_object)
                .and_then(|fields| fields.get(key)),
        ),
    }
}

fn owned(current: Option<&JsonValue>) -> OwnedValue {
    current.map_or(ABSENT, |value| OwnedValue {
        present: true,
        value: value.clone(),
    })
}

pub(super) fn env_is_empty(settings: &SettingsDocument) -> bool {
    match settings.get(ENV_KEY) {
        Some(JsonValue::Object(fields)) => fields.is_empty(),
        Some(
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::String(_)
            | JsonValue::Array(_),
        )
        | None => true,
    }
}

pub(super) fn with(settings: &mut SettingsDocument, path: &str, value: &OwnedValue) {
    let Some((section, key)) = path.rsplit_once('.') else {
        set_key(settings, path, value);
        return;
    };
    let mut env: SettingsDocument = settings
        .get(ENV_KEY)
        .and_then(JsonValue::as_object)
        .map(|fields| {
            fields
                .iter()
                .map(|(name, current)| (name.clone(), current.clone()))
                .collect()
        })
        .unwrap_or_default();
    set_key(&mut env, key, value);
    settings.insert(
        section.to_owned(),
        JsonValue::Object(env.into_iter().collect()),
    );
}

fn set_key(settings: &mut SettingsDocument, key: &str, value: &OwnedValue) {
    if value.present {
        settings.insert(key.to_owned(), value.value.clone());
    } else {
        settings.remove(key);
    }
}
