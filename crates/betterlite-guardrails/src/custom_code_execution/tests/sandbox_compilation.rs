use rstest::rstest;

use super::super::sandbox::DEFAULT_FILENAME;
use super::super::sandbox::compile_sandboxed;
use super::{compiled, failure};

#[test]
fn compiles_a_guardrail_program_and_returns_the_source() {
    let program =
        compiled("def apply_guardrail(inputs, request_data, input_type):\n    return allow()\n");
    assert!(
        program
            .source
            .contains("def apply_guardrail(inputs, request_data, input_type):")
    );
    assert_eq!(program.filename, DEFAULT_FILENAME);
    assert_eq!(program.functions.len(), 1);
    assert_eq!(program.functions.first().unwrap().name, "apply_guardrail");
    assert_eq!(program.is_async("apply_guardrail"), Some(false));
    assert_eq!(program.is_async("missing"), None);
}

#[rstest]
#[case::missing_paren("def broken(:\n    pass\n", "invalid syntax")]
#[case::import_statement("import os\n", "import statements are not allowed")]
#[case::import_from("from os import path\n", "import statements are not allowed")]
#[case::class_definition("class Filter:\n    pass\n", "class definitions are not allowed")]
#[case::exec_call("exec(\"x = 1\")\n", "Exec calls are not allowed")]
#[case::eval_call("eval(\"1 + 1\")\n", "Eval calls are not allowed")]
#[case::dunder_attribute("def f(x):\n    return x.__class__\n", "starts with \"_\"")]
#[case::dunder_name("def f():\n    return __import__\n", "starts with \"_\"")]
#[case::roles_name("def f(roles__):\n    return roles__\n", "__roles__")]
#[case::print_name("def f():\n    print(\"hi\")\n", "is a reserved name")]
#[case::nonlocal_statement(
    "def f():\n    def g():\n        nonlocal total\n",
    "nonlocal statements are not allowed"
)]
fn rejects_forbidden_constructs(#[case] source: &str, #[case] requirement: &str) {
    let error = compile_sandboxed(source, DEFAULT_FILENAME).unwrap_err();
    let (_, _, message) = failure(&error);
    assert!(
        message.contains(requirement),
        "expected {requirement:?} in {message:?}"
    );
}

#[rstest]
#[case::inspect_attribute("def f(x):\n    return x.f_back\n")]
#[case::inspect_code("def f(x):\n    return x.co_code\n")]
fn rejects_inspect_attributes(#[case] source: &str) {
    let error = compile_sandboxed(source, DEFAULT_FILENAME).unwrap_err();
    assert!(failure(&error).2.contains("restricted name"));
}

#[test]
fn reports_the_file_and_line_of_the_failure() {
    let error = compile_sandboxed("\n\nimport os\n", "guard.py").unwrap_err();
    assert_eq!(failure(&error), ("guard.py", 3, "invalid syntax"));
}

const RESPONSE_REJECTION_CODE: &str = r#"
def apply_guardrail(inputs, request_data, input_type):
    """Block responses that indicate the model rejected the user request."""
    if input_type != "response":
        return allow()

    texts = inputs.get("texts") or []
    rejection_phrases = [
        "i can't help with that",
        "i cannot help with that",
    ]

    for text in texts:
        if not text:
            continue
        text_lower = lower(text)
        for phrase in rejection_phrases:
            if contains(text_lower, phrase):
                return block(
                    "Response indicates the model rejected the user request.",
                    detection_info={"matched_phrase": phrase, "input_type": "response"},
                )
    return allow()
"#;

#[test]
fn accepts_the_live_response_rejection_program() {
    let program = compiled(RESPONSE_REJECTION_CODE);
    assert_eq!(program.is_async("apply_guardrail"), Some(false));
}
