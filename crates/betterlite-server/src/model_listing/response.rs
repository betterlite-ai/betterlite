use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;

const CHOICES: &str = "choices";

// _PRESERVED_NONE_FIELDS: the nulls an exclude_none dump strips while OpenAI still requires them.
pub const PRESERVED_NONE_FIELDS: [(&str, &str); 3] = [
    ("message", "content"),
    ("message", "role"),
    ("delta", "content"),
];

#[must_use]
// ast-grep-ignore: no-long-names
pub fn model_dump_with_preserved_fields(
    mut dumped: JsonObject,
    original_choices: &[JsonObject],
) -> JsonObject {
    let Some(Json::List(choices)) = dumped.get_mut(CHOICES) else {
        return dumped;
    };
    for (choice, original) in choices.iter_mut().zip(original_choices) {
        let Json::Object(choice_object) = choice else {
            continue;
        };
        for (sub_object, field) in PRESERVED_NONE_FIELDS {
            restore_field(choice_object, original, sub_object, field);
        }
    }
    dumped
}

// the field the original sub-object still carries, for a dumped sub-object that dropped it
fn restore_field(
    choice: &mut IndexMap<String, Json>,
    original: &JsonObject,
    sub_object: &str,
    field: &str,
) {
    let Some(Json::Object(sub_dict)) = choice.get_mut(sub_object) else {
        return;
    };
    if sub_dict.contains_key(field) {
        return;
    }
    let Some(Json::Object(original_sub)) = original.get(sub_object) else {
        return;
    };
    if let Some(value) = original_sub.get(field) {
        sub_dict.insert(field.to_owned(), value.clone());
    }
}
