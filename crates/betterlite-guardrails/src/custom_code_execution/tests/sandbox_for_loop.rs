use super::compiled;

#[test]
fn wraps_a_for_loop_iterable_with_getiter() {
    let program = compiled("for text in texts:\n    check(text)\n");
    assert!(
        program.source.contains("for text in _getiter_(texts):"),
        "{}",
        program.source
    );
}

#[test]
fn wraps_a_call_expression_used_as_the_iterable() {
    let program = compiled("for item in items():\n    pass\n");
    assert!(program.source.contains("for item in _getiter_(items()):"));
}

#[test]
fn wraps_the_iterable_of_an_async_for() {
    let program = compiled("async def f(stream):\n    async for chunk in stream:\n        pass\n");
    assert!(
        program
            .source
            .contains("async for chunk in _getiter_(stream):")
    );
}

#[test]
fn wraps_each_iterable_of_a_comprehension() {
    let program = compiled("values = [x for x in items if keep(x)]\n");
    assert!(
        program
            .source
            .contains("[x for x in _getiter_(items) if keep(x)]"),
        "{}",
        program.source
    );
}

#[test]
fn wraps_every_clause_of_a_nested_comprehension() {
    let program = compiled("values = [x + y for x in rows for y in columns]\n");
    assert!(
        program
            .source
            .contains("[x + y for x in _getiter_(rows) for y in _getiter_(columns)]")
    );
}

#[test]
fn wraps_set_dict_and_generator_comprehensions() {
    let program =
        compiled("a = {x for x in items}\nb = {k: v for k, v in items}\nc = (x for x in items)\n");
    let rendered = program.source;
    assert!(rendered.contains("{x for x in _getiter_(items)}"));
    assert!(rendered.contains("{k: v for k, v in _getiter_(items)}"));
    assert!(rendered.contains("(x for x in _getiter_(items))"));
}

#[test]
fn keeps_the_for_else_block() {
    let program = compiled("for item in items:\n    pass\nelse:\n    done()\n");
    assert!(
        program
            .source
            .contains("for item in _getiter_(items):\n    pass\nelse:\n    done()")
    );
}

#[test]
fn leaves_a_nested_for_body_rewritten_once() {
    let program = compiled("for a in rows:\n    for b in a:\n        pass\n");
    let rendered = program.source;
    assert!(rendered.contains("for a in _getiter_(rows):"));
    assert!(rendered.contains("for b in _getiter_(a):"));
}
