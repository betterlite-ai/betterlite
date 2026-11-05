#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UnsafeSecretNameError {
    #[error("invalid secret name {name:?}")]
    Invalid { name: String },
}

pub fn raise_if_unsafe_secret_name(secret_name: &str) -> Result<(), UnsafeSecretNameError> {
    let has_parent_segment = secret_name.split('/').any(|segment| segment == "..");
    let has_unsafe_character = secret_name.chars().any(|character| {
        matches!(character, '\u{0000}'..='\u{001f}' | '\u{007f}'..='\u{009f}' | ' ' | '\u{2028}' | '\u{2029}')
    });
    if has_parent_segment || has_unsafe_character {
        return Err(UnsafeSecretNameError::Invalid {
            name: secret_name.to_owned(),
        });
    }
    Ok(())
}

#[path = "secret_name/tests.rs"]
mod tests;
