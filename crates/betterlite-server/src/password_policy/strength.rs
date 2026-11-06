use betterlite_config::sections::password_policy::{CharacterClass, Settings};
use icu_properties::CodePointMapData;
use icu_properties::props::{GeneralCategory, GeneralCategoryGroup, NumericType};
use secrecy::{ExposeSecret as _, SecretString};

use super::error::PasswordError;

// Python's str.isalnum: a letter (category L*) or any character with a numeric value.
fn is_alphanumeric(character: char) -> bool {
    let category = CodePointMapData::<GeneralCategory>::new().get(character);
    GeneralCategoryGroup::Letter.contains(category)
        || CodePointMapData::<NumericType>::new().get(character) != NumericType::None
}

fn has_class(password: &str, class: CharacterClass) -> bool {
    match class {
        CharacterClass::Uppercase => password.chars().any(char::is_uppercase),
        CharacterClass::Lowercase => password.chars().any(char::is_lowercase),
        // Python's str.isdigit: Numeric_Type Decimal or Digit.
        CharacterClass::Number => password.chars().any(|character| {
            matches!(
                CodePointMapData::<NumericType>::new().get(character),
                NumericType::Decimal | NumericType::Digit
            )
        }),
        CharacterClass::Special => password
            .chars()
            .any(|character| !is_alphanumeric(character)),
    }
}

const fn requirement(class: CharacterClass) -> &'static str {
    match class {
        CharacterClass::Uppercase => "include an uppercase letter",
        CharacterClass::Lowercase => "include a lowercase letter",
        CharacterClass::Number => "include a number",
        CharacterClass::Special => "include a special character",
    }
}

#[must_use]
pub fn violations(password: &SecretString, settings: &Settings) -> Vec<String> {
    let text = password.expose_secret();
    let length = u32::try_from(text.chars().count()).unwrap_or(u32::MAX);
    let mut found = Vec::new();
    if length < settings.min_length {
        found.push(format!(
            "be at least {} characters long",
            settings.min_length
        ));
    }
    for class in &settings.required {
        if !has_class(text, *class) {
            found.push(requirement(*class).to_owned());
        }
    }
    found
}

pub fn validate(password: &SecretString, settings: &Settings) -> Result<(), PasswordError> {
    let found = violations(password, settings);
    if found.is_empty() {
        return Ok(());
    }
    Err(PasswordError::Policy {
        requirements: found.join(", "),
    })
}
