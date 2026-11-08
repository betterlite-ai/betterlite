use super::super::sandbox::DEFAULT_FILENAME;
use super::super::sandbox::compile_sandboxed;
use super::{compiled, failure};

#[test]
fn allows_async_definitions_and_awaits() {
    let program = compiled(
        "async def apply_guardrail(inputs, request_data, input_type):\n    response = await http_get(\"https://example.com\")\n    return allow()\n",
    );
    assert_eq!(program.is_async("apply_guardrail"), Some(true));
    assert!(program.source.contains("await http_get"));
}

#[test]
fn allows_async_for_and_async_with() {
    let program = compiled(
        "async def apply_guardrail(inputs, lock):\n    async for text in inputs:\n        pass\n    async with lock:\n        pass\n    return allow()\n",
    );
    assert!(program.source.contains("async for text in"));
    assert!(program.source.contains("async with lock"));
}

#[test]
fn async_loops_still_take_their_guards() {
    let program = compiled(
        "async def f(items):\n    async for item in items:\n        pass\n    while True:\n        break\n",
    );
    let rendered = program.source;
    assert!(rendered.contains("async for item in items:"));
    assert!(rendered.contains("while _budget_ok_() and (True):"));
}

#[test]
fn records_sync_functions_as_sync() {
    let program = compiled("def first():\n    pass\ndef second():\n    pass\n");
    let names: Vec<&str> = program
        .functions
        .iter()
        .map(|function| function.name.as_str())
        .collect();
    assert_eq!(names, ["first", "second"]);
    assert_eq!(program.is_async("first"), Some(false));
}

#[test]
fn imports_and_classes_stay_forbidden_inside_async_code() {
    let import =
        compile_sandboxed("async def f():\n    import os\n", DEFAULT_FILENAME).unwrap_err();
    assert!(
        failure(&import)
            .2
            .contains("import statements are not allowed")
    );
    let class = compile_sandboxed(
        "async def f():\n    class Inner:\n        pass\n",
        DEFAULT_FILENAME,
    )
    .unwrap_err();
    assert!(
        failure(&class)
            .2
            .contains("class definitions are not allowed")
    );
}
