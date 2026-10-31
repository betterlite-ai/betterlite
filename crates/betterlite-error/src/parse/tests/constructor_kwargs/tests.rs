#![cfg(test)]

use super::*;

#[rstest]
#[case::constructor_kwargs()]
fn error_facts_behavior_tests_behavior() {
    let candidates = BTreeMap::from([
        (
            String::from("response"),
            ProviderErrorBody {
                message: Field::Missing,
                error: Field::Missing,
                code: Field::Missing,
            },
        ),
        (
            String::from("body"),
            ProviderErrorBody {
                message: Field::Missing,
                error: Field::Missing,
                code: Field::Missing,
            },
        ),
        (
            String::from("message"),
            ProviderErrorBody {
                message: Field::Missing,
                error: Field::Missing,
                code: Field::Missing,
            },
        ),
    ]);
    let accepted = accepted_init_kwargs(&["response", "body"], &candidates);
    assert_eq!(accepted.len(), 2);
    assert!(accepted.contains_key("response"));
    assert!(accepted.contains_key("body"));
}
