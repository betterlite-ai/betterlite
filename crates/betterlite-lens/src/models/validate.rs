use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RecordError {
    #[error("{field} must be {min} to {max} characters long, got {actual}")]
    Length {
        field: &'static str,
        min: usize,
        max: usize,
        actual: usize,
    },
    #[error("{field} must hold {min} to {max} items, got {actual}")]
    Items {
        field: &'static str,
        min: usize,
        max: usize,
        actual: usize,
    },
    #[error("{field} is out of range")]
    Range { field: &'static str },
    #[error("{field} does not match its pattern")]
    Pattern { field: &'static str },
    #[error("Each check must have a unique ID")]
    DuplicateCheck,
    #[error("Describe expected behavior or add an enabled check")]
    NoBehavior,
    #[error("expected_behavior is reserved for the behavior description")]
    ReservedCheck,
}

pub trait Validate {
    fn validate(&self) -> Result<(), RecordError> {
        Ok(())
    }
}

pub fn check_length(
    field: &'static str,
    text: &str,
    min: usize,
    max: usize,
) -> Result<(), RecordError> {
    // Pydantic counts characters, not bytes
    let actual = text.chars().count();
    if (min..=max).contains(&actual) {
        Ok(())
    } else {
        Err(RecordError::Length {
            field,
            min,
            max,
            actual,
        })
    }
}

pub fn check_items<T>(
    field: &'static str,
    items: &[T],
    min: usize,
    max: usize,
) -> Result<(), RecordError>
where
    T: Validate,
{
    let actual = items.len();
    if !(min..=max).contains(&actual) {
        return Err(RecordError::Items {
            field,
            min,
            max,
            actual,
        });
    }
    items.iter().try_for_each(Validate::validate)
}
