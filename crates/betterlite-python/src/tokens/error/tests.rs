#![cfg(test)]

use betterlite_tokenizer::count::error::CountError;

use super::TokensError;

#[test]
fn tokens_error_texts_match_litellm() {
    assert_eq!(
        TokensError::from(CountError::TextAndMessages).to_string(),
        "text and messages cannot both be set"
    );
    assert_eq!(
        TokensError::from(CountError::MissingInput).to_string(),
        "Either text or messages must be provided"
    );
    assert_eq!(
        TokensError::from(CountError::ToolsWithText).to_string(),
        "tools or tool_choice cannot be set if using text"
    );
}

#[test]
fn tokens_error_keeps_the_renamed_arguments_as_their_own_variants() {
    assert!(matches!(
        TokensError::from(CountError::TextAndMessages),
        TokensError::TextAndMessages
    ));
    assert!(matches!(
        TokensError::from(CountError::MissingInput),
        TokensError::MissingInput
    ));
    assert!(matches!(
        TokensError::from(CountError::ToolsWithText),
        TokensError::ToolsWithText
    ));
}

#[test]
fn tokens_error_keeps_the_counter_errors_that_litellm_never_names() {
    let error = TokensError::from(CountError::Overflow);
    assert!(matches!(error, TokensError::Count(CountError::Overflow)));
    assert_eq!(error.to_string(), CountError::Overflow.to_string());
}
