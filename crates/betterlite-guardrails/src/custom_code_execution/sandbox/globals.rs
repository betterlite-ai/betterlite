use crate::primitives::registry::{Primitive, RegistryEntry, get_custom_code_primitives};
use indexmap::IndexMap;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Guard {
    Getattr,
    Getitem,
    Getiter,
    IterUnpackSequence,
    Write,
    Inplacevar,
    BudgetOk,
}
const GUARDS: [(&str, Guard); 7] = [
    ("_getattr_", Guard::Getattr),
    ("_getitem_", Guard::Getitem),
    ("_getiter_", Guard::Getiter),
    ("_iter_unpack_sequence_", Guard::IterUnpackSequence),
    ("_write_", Guard::Write),
    ("_inplacevar_", Guard::Inplacevar),
    ("_budget_ok_", Guard::BudgetOk),
];
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinKind {
    Safe,
    Limited,
    Utility,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Global {
    Primitive(Primitive),
    Builtin(BuiltinKind),
    Guard(Guard),
    Builtins,
}
const SAFE_BUILTINS: &str = "__build_class__ None False True Ellipsis abs bool bytes callable chr complex divmod float hash hex id int isinstance issubclass len oct ord pow range repr round slice sorted str tuple zip setattr delattr ArithmeticError AssertionError AttributeError BaseException BufferError BytesWarning DeprecationWarning EOFError EnvironmentError Exception FloatingPointError FutureWarning GeneratorExit IOError ImportError ImportWarning IndentationError IndexError KeyError KeyboardInterrupt LookupError MemoryError NameError NotImplementedError OSError OverflowError PendingDeprecationWarning ReferenceError RuntimeError RuntimeWarning StopIteration SyntaxError SyntaxWarning SystemError SystemExit TabError TypeError UnboundLocalError UnicodeDecodeError UnicodeEncodeError UnicodeError UnicodeTranslateError UnicodeWarning UserWarning ValueError Warning ZeroDivisionError";
const LIMITED_BUILTINS: &str = " range list tuple";
const UTILITY_BUILTINS: &str = " string math random whrandom set frozenset same_type test reorder";
#[must_use]
pub fn build_sandbox_builtins() -> IndexMap<&'static str, BuiltinKind> {
    let mut builtins: IndexMap<&'static str, BuiltinKind> = SAFE_BUILTINS
        .split_whitespace()
        .map(|name| (name, BuiltinKind::Safe))
        .collect();
    builtins.extend(
        LIMITED_BUILTINS
            .split_whitespace()
            .map(|name| (name, BuiltinKind::Limited)),
    );
    builtins.extend(
        UTILITY_BUILTINS
            .split_whitespace()
            .map(|name| (name, BuiltinKind::Utility)),
    );
    builtins
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxGlobals {
    pub entries: IndexMap<&'static str, Global>,
    pub builtins: IndexMap<&'static str, BuiltinKind>,
}
#[must_use]
pub fn build_sandbox_globals() -> SandboxGlobals {
    let mut entries = IndexMap::new();
    for (name, entry) in get_custom_code_primitives() {
        let global = match entry {
            RegistryEntry::Function(primitive) => Global::Primitive(primitive),
            RegistryEntry::Builtin(_builtin) => Global::Builtin(BuiltinKind::Safe),
        };
        entries.insert(name, global);
    }
    for (name, guard) in GUARDS {
        entries.insert(name, Global::Guard(guard));
    }
    entries.insert("__builtins__", Global::Builtins);
    SandboxGlobals {
        entries,
        builtins: build_sandbox_builtins(),
    }
}
