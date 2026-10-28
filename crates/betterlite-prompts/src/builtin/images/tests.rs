#![cfg(test)]

use super::convert_to_ollama_image;
use crate::builtin::PromptError;
use rstest::rstest;

#[rstest]
#[case::inline("data:image/png;base64,QQ==", Some("QQ=="))]
#[case::raw("QQ==", Some("QQ=="))]
#[case::empty("", Some(""))]
#[case::repeated_marker("data:image/png;base64,first;base64,second", Some("first"))]
#[case::malformed("data:image/png", None)]
#[case::remote("https://example.test/image", None)]
fn builtin_prompts_behavior_tests_behavior(#[case] input: &str, #[case] expected: Option<&str>) {
    let result = convert_to_ollama_image(input);
    match expected {
        Some(text) => assert_eq!(result.unwrap(), text),
        None => assert!(matches!(
            result,
            Err(PromptError::Image | PromptError::RemoteImage)
        )),
    }
}
