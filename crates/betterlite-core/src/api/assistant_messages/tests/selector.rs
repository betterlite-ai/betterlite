use betterlite_prompts::arguments::json::Object;

use super::{json, object, options, selected};

#[test]
fn message_option_omissions() {
    assert_eq!(
        selected(&options(r#"{"provider":"openai"}"#)),
        Object::new()
    );
    assert_eq!(
        selected(&options(
            r#"{"provider":"openai","role":null,"content":null,"attachments":null,"metadata":null}"#
        )),
        Object::new()
    );
    assert_eq!(
        selected(&options(
            r#"{"provider":"openai","role":"user","content":"","attachments":[],"metadata":{}}"#
        )),
        object(r#"{"role":"user","content":"","attachments":[],"metadata":{}}"#)
    );
    let openai = selected(&options(
        r#"{"provider":"openai","role":"user","content":"hi","kwargs":{"trace":"x"}}"#,
    ));
    assert_eq!(openai.get("trace"), Some(&json(r#""x""#)));
    assert!(openai.contains_key("role"));
    let azure = selected(&options(
        r#"{"provider":"azure","role":"user","content":"hi","kwargs":{"trace":"x"}}"#,
    ));
    assert_eq!(azure.get("trace"), Some(&json(r#""x""#)));
    assert!(azure.contains_key("role"));
    let unknown = selected(&options(
        r#"{"provider":"unknown","role":"user","content":"hi","kwargs":{"trace":"x"}}"#,
    ));
    assert_eq!(unknown.get("trace"), Some(&json(r#""x""#)));
    assert!(!unknown.contains_key("role"));
    assert_eq!(
        selected(&options(
            r#"{"provider":"anthropic","role":"user","content":"hi","kwargs":{"trace":"x"}}"#
        )),
        object(r#"{"trace":"x"}"#)
    );
}

#[test]
fn message_option_omissions_keeps_the_system_role() {
    assert_eq!(
        selected(&options(
            r#"{"provider":"openai","role":"system","content":"hi"}"#
        )),
        object(r#"{"role":"system","content":"hi"}"#)
    );
}
