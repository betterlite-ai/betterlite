#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenRouterPrefix {
    Nested { model: String },
    Native { model: String },
    NotApplicable,
}

#[must_use]
pub fn normalize_openrouter_prefix(
    model: &str,
    custom_llm_provider: Option<&str>,
) -> OpenRouterPrefix {
    if custom_llm_provider != Some("openrouter") {
        return OpenRouterPrefix::NotApplicable;
    }
    let Some(remainder) = model.strip_prefix("openrouter/") else {
        return OpenRouterPrefix::NotApplicable;
    };
    if remainder.contains('/') {
        OpenRouterPrefix::Nested {
            model: remainder.to_owned(),
        }
    } else {
        OpenRouterPrefix::Native {
            model: model.to_owned(),
        }
    }
}

mod tests;
