#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_env::EnvError;
use rstest::rstest;

use crate::sections::Sections;

const AUTO_TRUNCATE: &str = "VERTEX_AI_TEXT_EMBEDDING_AUTO_TRUNCATE";
const TASK_TYPE: &str = "VERTEX_AI_TEXT_EMBEDDING_TASK_TYPE";
const TITLE: &str = "VERTEX_AI_TEXT_EMBEDDING_TITLE";

type Loaded = (Option<bool>, Option<String>, Option<String>);

fn lookup(environment: &[(&str, &str)]) -> BTreeMap<String, String> {
    environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn load(environment: &[(&str, &str)]) -> Result<Loaded, EnvError> {
    Sections::vertex_text_embedding_settings(&lookup(environment))
}

#[rstest]
#[case::absent(&[], (None, None, None))]
#[case::auto_truncate(&[(AUTO_TRUNCATE, "true")], (Some(true), None, None))]
#[case::auto_truncate_off(&[(AUTO_TRUNCATE, "0")], (Some(false), None, None))]
#[case::auto_truncate_yes(&[(AUTO_TRUNCATE, " yes ")], (Some(true), None, None))]
#[case::auto_truncate_empty(&[(AUTO_TRUNCATE, "")], (None, None, None))]
#[case::task_type(&[(TASK_TYPE, "RETRIEVAL_DOCUMENT")], (None, Some("RETRIEVAL_DOCUMENT"), None))]
#[case::task_type_empty(&[(TASK_TYPE, "")], (None, None, None))]
#[case::title(&[(TITLE, "annual report")], (None, None, Some("annual report")))]
#[case::title_empty(&[(TITLE, "")], (None, None, None))]
#[case::everything(
    &[(AUTO_TRUNCATE, "false"), (TASK_TYPE, "CLUSTERING"), (TITLE, "docs")],
    (Some(false), Some("CLUSTERING"), Some("docs"))
)]
fn settings_load_explicit_and_absent_values(
    #[case] environment: &[(&str, &str)],
    #[case] expected: (Option<bool>, Option<&str>, Option<&str>),
) {
    let (auto_truncate, task_type, title) = load(environment).unwrap();
    assert_eq!(auto_truncate, expected.0);
    assert_eq!(task_type.as_deref(), expected.1);
    assert_eq!(title.as_deref(), expected.2);
}

#[test]
fn an_invalid_auto_truncate_names_the_variable() {
    let error = load(&[(AUTO_TRUNCATE, "maybe")]).unwrap_err();
    assert_eq!(
        error,
        EnvError::Bool {
            name: AUTO_TRUNCATE.to_owned()
        }
    );
}
