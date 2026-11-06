#![cfg(test)]

use crate::arguments::json::Object;
use crate::gitlab_client::tests::answer;
use crate::gitlab_templates::error::ManagerError;
use crate::gitlab_templates::tests::{SCOPED_CONFIG, build_manager};

fn variables(json_text: &str) -> Object {
    serde_json::from_str(json_text).unwrap()
}

#[tokio::test]
async fn render_template_renders_the_stored_template_with_the_variables() {
    let file = "---\nmodel: gpt-4\n---\nHello {{ name }}, you have {{ count }} items.";
    let (built, _) = build_manager(SCOPED_CONFIG, Some("a"), None, vec![answer!(200, file)]).await;
    let manager = built.ok().unwrap();
    let rendered = manager
        .render_template("a", &variables(r#"{"name": "Ada", "count": 3}"#))
        .unwrap();
    assert_eq!(rendered, "Hello Ada, you have 3 items.");
}

#[tokio::test]
async fn render_template_without_variables_leaves_missing_ones_empty() {
    let (built, _) = build_manager(
        SCOPED_CONFIG,
        Some("a"),
        None,
        vec![answer!(200, "Hi {{ name }}!")],
    )
    .await;
    let rendered = built.ok().unwrap().render_template("a", &Object::new());
    assert_eq!(rendered.unwrap(), "Hi !");
}

#[tokio::test]
async fn an_unknown_template_id_has_the_exact_not_found_message() {
    let (built, _) = build_manager(SCOPED_CONFIG, None, None, vec![]).await;
    let error = built
        .ok()
        .unwrap()
        .render_template("gitlab::missing", &Object::new())
        .unwrap_err();
    assert!(matches!(error, ManagerError::NotFound(_)));
    assert_eq!(error.to_string(), "Template 'gitlab::missing' not found");
}

#[tokio::test]
async fn the_template_id_is_looked_up_exactly_as_it_was_loaded() {
    let (built, _) = build_manager(
        SCOPED_CONFIG,
        Some("chat/hi"),
        None,
        vec![answer!(200, "x")],
    )
    .await;
    let manager = built.ok().unwrap();
    manager.render_template("chat/hi", &Object::new()).unwrap();
    manager
        .render_template("gitlab::chat::hi", &Object::new())
        .unwrap_err();
}

#[tokio::test]
async fn get_template_returns_the_stored_template_or_none() {
    let file = "---\nmodel: gpt-4\ntop_p: 0.9\n---\nBody";
    let (built, _) = build_manager(SCOPED_CONFIG, Some("a"), None, vec![answer!(200, file)]).await;
    let manager = built.ok().unwrap();
    let template = manager.prompts.get("a").unwrap();
    assert_eq!(template.model.as_deref(), Some("gpt-4"));
    assert_eq!(template.content, "Body");
    assert!(template.optional_params.contains_key("top_p"));
    assert!(manager.prompts.get("b").is_none());
}

#[tokio::test]
async fn a_template_syntax_error_is_a_render_error_naming_the_template() {
    let (built, _) = build_manager(
        SCOPED_CONFIG,
        Some("a"),
        None,
        vec![answer!(200, "{% if %}")],
    )
    .await;
    let error = built
        .ok()
        .unwrap()
        .render_template("a", &Object::new())
        .unwrap_err();
    assert!(matches!(error, ManagerError::Render(_)));
    assert!(
        error
            .to_string()
            .starts_with("Error rendering template 'a': ")
    );
}

#[tokio::test]
async fn variables_are_not_validated_against_the_input_schema() {
    let file = "---\ninput:\n  schema:\n    name: string\n---\nHi {{ name }}";
    let (built, _) = build_manager(SCOPED_CONFIG, Some("a"), None, vec![answer!(200, file)]).await;
    let manager = built.ok().unwrap();
    assert_eq!(manager.render_template("a", &Object::new()).unwrap(), "Hi ");
    let wrong_type = variables(r#"{"name": 5}"#);
    assert_eq!(manager.render_template("a", &wrong_type).unwrap(), "Hi 5");
}
