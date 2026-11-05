use super::{RegistryEntry, get_custom_code_primitives};

#[derive(Debug, PartialEq, Eq)]
enum Kind {
    Function,
    Builtin,
    Absent,
}

const FUNCTIONS: [&str; 29] = [
    "allow",
    "block",
    "flag",
    "modify",
    "regex_match",
    "regex_match_all",
    "regex_replace",
    "regex_find_all",
    "json_parse",
    "json_stringify",
    "json_schema_valid",
    "extract_urls",
    "is_valid_url",
    "all_urls_valid",
    "get_url_domain",
    "http_request",
    "http_get",
    "http_post",
    "detect_code",
    "detect_code_languages",
    "contains_code_language",
    "contains",
    "contains_any",
    "contains_all",
    "word_count",
    "char_count",
    "lower",
    "upper",
    "trim",
];

#[rstest::rstest]
#[case::every_function(&FUNCTIONS, Kind::Function)]
#[case::safe_builtins(&["len", "str", "int", "float", "bool", "list", "dict", "True", "False", "None"], Kind::Builtin)]
#[case::nothing_unsafe(&["eval", "exec", "open", "__import__", "getattr", "compile", "input"], Kind::Absent)]
fn primitives_registry(#[case] names: &[&str], #[case] expected: Kind) {
    let registry = get_custom_code_primitives();
    assert_eq!(registry.len(), 39);
    assert_eq!(
        registry.keys().copied().take(4).collect::<Vec<_>>(),
        ["allow", "block", "flag", "modify"]
    );
    for name in names {
        let found = match registry.get(name) {
            Some(RegistryEntry::Function(primitive)) => {
                assert_eq!(primitive.signature().0, *name);
                Kind::Function
            }
            Some(RegistryEntry::Builtin(builtin)) => {
                assert_eq!(builtin.name(), *name);
                Kind::Builtin
            }
            None => Kind::Absent,
        };
        assert_eq!(found, expected, "{name}");
    }
}
