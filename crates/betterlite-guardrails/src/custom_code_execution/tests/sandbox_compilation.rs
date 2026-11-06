use std::thread;

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
#[case::roles_name("def f(a__roles__):\n    return a__roles__\n", "__roles__")]
#[case::nonlocal_statement(
    "def f():\n    def g():\n        nonlocal total\n",
    "nonlocal statements are not allowed"
)]
#[case::dunder_object("def f():\n    return __builtins__.values()\n", "starts with \"_\"")]
#[case::dunder_object_two("_secret = 1\n", "starts with \"_\"")]
#[case::dunder_object_three("def f(y):\n    return _secret.y\n", "starts with \"_\"")]
#[case::parenthesized_exec("(exec)(\"x = 1\")\n", "Exec calls are not allowed")]
#[case::print_statement("print \"x\"\n", "print statements are not allowed")]
#[case::annotated_assignment("x: int = 1\n", "annotated assignment is not allowed")]
#[case::match_statement(
    "match x:\n    case 1:\n        pass\n",
    "match statements are not allowed"
)]
#[case::augmented_attribute("def f(x):\n    x.y += 1\n", "Augmented assignment of attributes")]
#[case::augmented_subscript("def f(x):\n    x[0] += 1\n", "Augmented assignment of object items")]
#[case::return_outside("return 1\n", "return outside function")]
#[case::break_outside("break\n", "loop control outside loop")]
#[case::yield_outside("yield 1\n", "yield outside function")]
#[case::await_outside("def f():\n    await g()\n", "await outside async function")]
#[case::duplicate_parameter("def f(a, a):\n    pass\n", "duplicate parameter")]
#[case::default_before_plain("def f(a=1, b):\n    pass\n", "non-default parameter")]
#[case::positional_after_keyword("f(a=1, 2)\n", "positional after a keyword argument")]
#[case::comprehension_tuple("values = [x for x in a, b]\n", "single iterable")]
#[case::comparison_python_two("def f(a, b):\n    return a <> b\n", "Python 2")]
#[case::except_star("try:\n    pass\nexcept* E:\n    pass\n", "except* is not allowed")]
#[case::nfkc_exec(
    "\u{FF45}\u{FF58}\u{FF45}\u{FF43}(\"1\")\n",
    "Exec calls are not allowed"
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
    let error = compile_sandboxed("\n\ndef broken(:\n", "guard.py").unwrap_err();
    assert_eq!(failure(&error), ("guard.py", 3, "invalid syntax"));
}

#[test]
fn a_print_call_and_keyword_names_compile_as_in_litellm() {
    let program =
        compiled("def debug(x):\n    print(x)\n\ndef f():\n    return g(print=1, _x=2)\n");
    assert_eq!(program.functions.len(), 2);
}

#[test]
fn a_nested_magic_method_compiles_and_a_module_one_does_not() {
    let program = compiled("def outer():\n    def __lt__(self, other):\n        return True\n");
    assert_eq!(program.functions.first().unwrap().name, "outer");
    let error = compile_sandboxed(
        "def __lt__(self, other):\n    return True\n",
        DEFAULT_FILENAME,
    )
    .unwrap_err();
    assert!(failure(&error).2.contains("starts with \"_\""));
}

#[test]
fn a_decorated_and_a_conditional_def_bind_module_functions() {
    let program = compiled(
        "@decorate\ndef first():\n    pass\n\nif True:\n    def apply_guardrail(a):\n        return allow()\n",
    );
    let names: Vec<&str> = program
        .functions
        .iter()
        .map(|function| function.name.as_str())
        .collect();
    assert_eq!(names, ["first", "apply_guardrail"]);
    assert_eq!(program.is_async("apply_guardrail"), Some(false));
}

#[test]
fn a_later_definition_replaces_an_earlier_one() {
    let program = compiled(
        "def apply_guardrail(a):\n    return allow()\n\nasync def apply_guardrail(a):\n    return allow()\n",
    );
    assert_eq!(program.functions.len(), 1);
    assert_eq!(program.is_async("apply_guardrail"), Some(true));
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

#[test]
fn deeply_nested_invalid_source_does_not_overflow_a_small_stack() {
    let source = format!("{}1 +\n{}", "(".repeat(20_000), ")".repeat(20_000));
    let worker = thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(move || compile_sandboxed(&source, DEFAULT_FILENAME).is_err())
        .unwrap();
    assert!(worker.join().unwrap());
}
