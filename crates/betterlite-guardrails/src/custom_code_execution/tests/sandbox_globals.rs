use rstest::rstest;

use super::super::sandbox::globals::{
    BuiltinKind, Global, Guard, build_sandbox_builtins, build_sandbox_globals,
};
use crate::primitives::registry::{ActionPrimitive, Primitive, TextPrimitive};

#[rstest]
#[case::getattr("_getattr_", Guard::Getattr)]
#[case::getitem("_getitem_", Guard::Getitem)]
#[case::getiter("_getiter_", Guard::Getiter)]
#[case::iter_unpack("_iter_unpack_sequence_", Guard::IterUnpackSequence)]
#[case::write("_write_", Guard::Write)]
#[case::inplacevar("_inplacevar_", Guard::Inplacevar)]
#[case::budget("_budget_ok_", Guard::BudgetOk)]
fn exposes_every_restricted_guard(#[case] name: &str, #[case] expected: Guard) {
    let globals = build_sandbox_globals();
    assert_eq!(globals.entries.get(name), Some(&Global::Guard(expected)));
    assert_eq!(
        globals
            .entries
            .values()
            .filter(|global| matches!(global, Global::Guard(_)))
            .count(),
        7
    );
}

#[test]
fn exposes_the_primitive_registry() {
    let globals = build_sandbox_globals();
    assert_eq!(
        globals.entries.get("allow"),
        Some(&Global::Primitive(Primitive::Action(
            ActionPrimitive::Allow
        )))
    );
    assert_eq!(
        globals.entries.get("block"),
        Some(&Global::Primitive(Primitive::Action(
            ActionPrimitive::Block
        )))
    );
    assert_eq!(
        globals.entries.get("lower"),
        Some(&Global::Primitive(Primitive::Text(TextPrimitive::Lower)))
    );
    assert_eq!(
        globals
            .entries
            .values()
            .filter(|global| matches!(global, Global::Primitive(_)))
            .count(),
        29
    );
    assert_eq!(globals.entries.get("__builtins__"), Some(&Global::Builtins));
}

#[test]
fn exposes_the_safe_builtins_and_overrides_the_limited_ones() {
    let globals = build_sandbox_globals();
    let builtins = &globals.builtins;
    assert_eq!(builtins.get("len"), Some(&BuiltinKind::Safe));
    assert_eq!(builtins.get("sorted"), Some(&BuiltinKind::Safe));
    assert_eq!(builtins.get("ValueError"), Some(&BuiltinKind::Safe));
    assert_eq!(builtins.get("range"), Some(&BuiltinKind::Limited));
    assert_eq!(builtins.get("list"), Some(&BuiltinKind::Limited));
    assert_eq!(builtins.get("tuple"), Some(&BuiltinKind::Limited));
    assert_eq!(builtins.get("math"), Some(&BuiltinKind::Utility));
    assert_eq!(builtins.get("set"), Some(&BuiltinKind::Utility));
    assert_eq!(builtins.get("open"), None);
    assert_eq!(builtins.get("eval"), None);
}

#[test]
fn the_standalone_builtins_match_the_globals_namespace() {
    let builtins = build_sandbox_builtins();
    assert_eq!(builtins.len(), 90);
    assert_eq!(builtins.get("range"), Some(&BuiltinKind::Limited));
}
