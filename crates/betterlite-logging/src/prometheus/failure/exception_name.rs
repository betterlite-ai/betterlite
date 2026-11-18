use betterlite_error::normalize::class::Class;

use super::exception::LabelException;

#[must_use]
pub fn exception_class_name(exception: &LabelException) -> String {
    if let Some(legacy) = exception
        .legacy_class_name
        .as_deref()
        .filter(|name| !name.is_empty())
    {
        return legacy.to_owned();
    }
    if matches!(exception.class, Class::BudgetExceededError) {
        return String::from("BudgetExceededError");
    }
    let provider_name = exception
        .llm_provider
        .as_deref()
        .filter(|provider| !provider.is_empty());
    let Some(provider) = provider_name else {
        return exception.class_name.clone();
    };
    let first_end = provider
        .char_indices()
        .nth(1)
        .map_or(provider.len(), |(index, _character)| index);
    let (first, rest) = provider.split_at(first_end);
    format!("{}{}.{}", first.to_uppercase(), rest, exception.class_name)
}
