use super::compiled;

#[test]
fn guards_the_while_test_and_keeps_the_original_condition() {
    let program =
        compiled("def f(xs):\n    i = 0\n    while i < len(xs) and not done(i):\n        i += 1\n");
    assert!(
        program
            .source
            .contains("while _budget_ok_() and i < len(xs) and not done(i):"),
        "{}",
        program.source
    );
}

#[test]
fn guards_every_while_in_a_nested_program() {
    let program = compiled("while outer:\n    while inner:\n        pass\n");
    let rendered = program.source;
    assert!(rendered.contains("while _budget_ok_() and outer:"));
    assert!(rendered.contains("while _budget_ok_() and inner:"));
}

#[test]
fn keeps_the_else_block_after_the_body() {
    let program = compiled("while running:\n    pass\nelse:\n    done()\n");
    assert!(
        program
            .source
            .contains("while _budget_ok_() and running:\n    pass\nelse:\n    done()"),
        "{}",
        program.source
    );
}

#[test]
fn guards_a_condition_that_spans_lines() {
    let program = compiled("while (first\n       and second):\n    pass\n");
    assert!(
        program
            .source
            .contains("while _budget_ok_() and (first\n       and second):")
    );
}

#[test]
fn guards_a_while_inside_an_async_function() {
    let program = compiled("async def f():\n    while waiting():\n        await drain()\n");
    assert!(
        program
            .source
            .contains("while _budget_ok_() and waiting():")
    );
}
