use super::error::ClassifierVerdictError;

pub const MAX_PROMPT_CHARS: usize = 2000;
pub const MAX_EXAMPLES_CHARS: usize = 4000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperatorSection {
    Prompt,
    Examples,
}

impl OperatorSection {
    #[must_use]
    pub const fn name_and_cap(self) -> (&'static str, usize) {
        match self {
            Self::Prompt => ("classification_prompt", MAX_PROMPT_CHARS),
            Self::Examples => ("classification_examples", MAX_EXAMPLES_CHARS),
        }
    }
}

pub fn normalize_section(
    section: OperatorSection,
    value: Option<&str>,
) -> Result<Option<String>, ClassifierVerdictError> {
    let (field, cap) = section.name_and_cap();
    let Some(value) = value else {
        return Ok(None);
    };
    let stripped = value.trim();
    if stripped.is_empty() {
        return Err(ClassifierVerdictError::OperatorSectionBlank { field });
    }
    if stripped.chars().count() > cap {
        return Err(ClassifierVerdictError::OperatorSectionTooLong { field, cap });
    }
    Ok(Some(stripped.to_owned()))
}
