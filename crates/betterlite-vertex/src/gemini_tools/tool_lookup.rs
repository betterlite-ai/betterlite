use betterlite_prompts::arguments::json::{Json, Object};

#[must_use]
pub fn underscore_name(name: &str) -> String {
    let mut underscored = String::with_capacity(name.len().saturating_add(4));
    for character in name.chars() {
        if character.is_uppercase() {
            underscored.push('_');
            underscored.extend(character.to_lowercase());
        } else {
            underscored.push(character);
        }
    }
    underscored.trim_start_matches('_').to_owned()
}
#[must_use]
pub fn tool_value<'tool>(tool: &'tool Object, name: &str) -> Option<&'tool Json> {
    let snake_name = underscore_name(name);
    let camel = tool.get(name).filter(|value| !matches!(value, Json::Null));
    let snake = tool
        .get(&snake_name)
        .filter(|value| !matches!(value, Json::Null));
    camel.or(snake)
}
