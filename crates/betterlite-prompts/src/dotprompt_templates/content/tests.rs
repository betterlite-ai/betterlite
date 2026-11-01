#![cfg(test)]

use super::dotprompt_content_to_json;

#[test]
fn splits_frontmatter_metadata_from_the_template_body() {
    let content = "---\nmodel: gpt-4\n---\nHello {{ name }}!";
    let result = dotprompt_content_to_json(content).unwrap();
    assert_eq!(result["content"].as_str(), Some("Hello {{ name }}!"));
    assert_eq!(
        result["metadata"].as_object().unwrap()["model"].as_str(),
        Some("gpt-4")
    );
}

#[test]
fn a_file_without_frontmatter_has_empty_metadata() {
    let result = dotprompt_content_to_json("Just plain content").unwrap();
    assert_eq!(result["content"].as_str(), Some("Just plain content"));
    assert!(result["metadata"].as_object().unwrap().is_empty());
}

#[test]
fn invalid_yaml_frontmatter_raises_immediately() {
    let content = "---\nmodel: [unterminated\n---\nbody";
    dotprompt_content_to_json(content).unwrap_err();
}
