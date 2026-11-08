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
#[case::typed_duplicate_parameter("def f(a: int, a: str):\n    pass\n", "duplicate parameter")]
#[case::typed_default_before_plain("def f(a=1, b: int):\n    pass\n", "non-default parameter")]
#[case::comprehension_trailing_comma("values = [x for x in a,]\n", "single iterable")]
#[case::async_for_outside("async for x in y:\n    pass\n", "async code outside async function")]
#[case::async_with_outside("async with a:\n    pass\n", "async code outside async function")]
#[case::async_comprehension_outside(
    "def f():\n    return [x async for x in y]\n",
    "asynchronous comprehension outside"
)]
#[case::await_in_lambda(
    "async def f():\n    return lambda: await y\n",
    "await outside async function"
)]
#[case::yield_from_in_async(
    "async def f():\n    yield from g()\n",
    "yield from inside async function"
)]
#[case::return_in_async_generator(
    "async def f():\n    yield 1\n    return 2\n",
    "return with value in async generator"
)]
#[case::yield_in_comprehension(
    "def f():\n    return [(yield x) for x in y]\n",
    "yield inside a comprehension"
)]
#[case::break_in_loop_else(
    "for x in y:\n    pass\nelse:\n    break\n",
    "loop control outside loop"
)]
#[case::type_alias("type X = int\n", "TypeAlias statements are not allowed")]
#[case::type_parameter("def f[T](x):\n    pass\n", "TypeVar statements are not allowed")]
#[case::print_assignment("print = 1\n", "reserved name")]
#[case::printed_assignment("printed = 1\n", "reserved name")]
#[case::print_function("def print(x):\n    pass\n", "reserved name")]
#[case::print_parameter("def f(print):\n    pass\n", "reserved name")]
#[case::print_default_parameter("def f(print=1):\n    pass\n", "reserved name")]
#[case::print_for_target("for print in x:\n    pass\n", "reserved name")]
#[case::print_lambda_parameter("f = lambda print: 1\n", "reserved name")]
#[case::print_with_target("with a as print:\n    pass\n", "reserved name")]
#[case::print_except_target("try:\n    pass\nexcept E as print:\n    pass\n", "reserved name")]
#[case::del_print("del print\n", "reserved name")]
#[case::del_list_with_print("del x, print\n", "reserved name")]
#[case::del_parenthesized_print("del (print)\n", "reserved name")]
#[case::print_walrus("(print := 1)\n", "reserved name")]
#[case::printed_walrus_in_if("if (printed := 1):\n    pass\n", "reserved name")]
#[case::print_walrus_in_fstring("f\"{(print := 1)}\"\n", "reserved name")]
#[case::print_in_with_tuple("with a as (print, b):\n    pass\n", "reserved name")]
#[case::print_in_with_list("with a as [print]:\n    pass\n", "reserved name")]
#[case::print_in_with_parentheses("with a as (print):\n    pass\n", "reserved name")]
#[case::yield_in_default("def h(x=(yield)):\n    pass\n", "yield outside function")]
#[case::yield_in_lambda_default("f = lambda x=(yield): x\n", "yield outside function")]
#[case::yield_in_annotation("def f(x: (yield)):\n    pass\n", "yield outside function")]
#[case::yield_in_return_annotation("def h() -> (yield):\n    pass\n", "yield outside function")]
#[case::await_in_default("async def h(x=await y):\n    pass\n", "await outside async function")]
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

#[rstest]
#[case::list_annotation("def f(x: list[int]):\n    return x\n")]
#[case::dict_return_annotation("def f() -> dict[str, int]:\n    return {}\n")]
#[case::optional_default("def f(x: Optional[str] = None):\n    return x\n")]
#[case::async_guardrail_signature(
    "async def apply_guardrail(inputs: list[str]) -> dict[str, str]:\n    return {}\n"
)]
#[case::global_print("def f():\n    global print\n")]
#[case::global_printed("def f():\n    global printed\n")]
#[case::await_in_nested_default("async def g():\n    def h(x=await y):\n        pass\n")]
#[case::await_in_lambda_default("async def g():\n    return lambda x=await y: x\n")]
fn accepts_what_restricted_python_compiles(#[case] source: &str) {
    assert!(!compiled(source).source.is_empty());
}

#[test]
fn a_print_call_and_keyword_names_compile_as_in_litellm() {
    let program =
        compiled("def debug(x):\n    print(x)\n\ndef f():\n    return g(print=1, _x=2)\n");
    assert_eq!(program.functions.len(), 2);
}

#[test]
fn print_and_printed_loads_compile_as_in_litellm() {
    let program = compiled(
        "def f():\n    return print\n\ndef g():\n    return printed\n\ndef h(a=print):\n    return a\n",
    );
    assert_eq!(program.functions.len(), 3);
}

#[rstest]
#[case::vararg("def f(a=1, *args):\n    pass\n")]
#[case::kwarg("def f(a=1, **kw):\n    pass\n")]
#[case::bare_separator("def f(a=1, *, b):\n    pass\n")]
#[case::keyword_defaults("def f(a, *, b=1, c):\n    pass\n")]
#[case::typed_vararg("def f(a=1, *args: int, b):\n    pass\n")]
#[case::helper("def h(x, y=None, **kw):\n    return x\n")]
fn accepts_splat_and_keyword_only_parameters(#[case] source: &str) {
    assert_eq!(compiled(source).functions.len(), 1);
}

#[rstest]
#[case::same_line("def f():\n    return g(a=1,  # why\n        b=2)\n")]
#[case::own_line(
    "def f():\n    return block(\n        \"msg\",\n        detection_info={\"a\": 1},  # note\n    )\n"
)]
fn accepts_comments_after_keyword_arguments(#[case] source: &str) {
    assert_eq!(compiled(source).functions.len(), 1);
}

#[rstest]
#[case::after("x = [[a for a in b], c]\n")]
#[case::before("x = [1, [a for a in b]]\n")]
#[case::dict_item("x = [{k: v for k, v in b}, 1]\n")]
#[case::set_item("x = [{a for a in b}, 1]\n")]
fn accepts_comprehensions_as_list_items(#[case] source: &str) {
    assert!(compiled(source).functions.is_empty());
}

#[rstest]
#[case::binding("_ = 1\n")]
#[case::unpacking("_, x = g()\n")]
#[case::augmented("_ += 1\n")]
#[case::attribute("_.x\n")]
#[case::comment("x = 1  # __roles__")]
fn accepts_underscore_names_and_trailing_comments(#[case] source: &str) {
    assert!(compiled(source).functions.is_empty());
}

#[test]
fn a_lambda_is_a_function_scope() {
    assert!(compiled("x = lambda: (yield)\n").functions.is_empty());
}

#[test]
fn a_comprehension_iterable_stays_in_the_enclosing_scope() {
    assert_eq!(
        compiled("def f():\n    return [x for x in (yield 1)]\n")
            .functions
            .len(),
        1
    );
}

#[test]
fn async_forms_compile_inside_an_async_function() {
    let program = compiled(
        "async def f():\n    return [x async for x in y]\n\nasync def g():\n    async for x in y:\n        pass\n    async with a:\n        pass\n",
    );
    assert_eq!(program.functions.len(), 2);
    assert_eq!(program.is_async("g"), Some(true));
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
    let arithmetic = format!("{}1 +\n{}", "(".repeat(20_000), ")".repeat(20_000));
    let called = format!("{}x{}()", "(".repeat(20_000), ")".repeat(20_000));
    let worker = thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(move || {
            let unclosed = compile_sandboxed(&arithmetic, DEFAULT_FILENAME).is_err();
            let parenthesized = compile_sandboxed(&called, DEFAULT_FILENAME).is_err();
            unclosed && parenthesized
        })
        .unwrap();
    assert!(worker.join().unwrap());
}
