use crate::mantle_responses::auth::{EnvironmentParams, PROJECT_HEADER, validate_environment};

use super::support;

pub(super) fn openai_project_header() {
    overwrite_cases();
    unchanged_cases();
}

fn overwrite_cases() {
    let globals = support::globals();
    let cases = [
        (PROJECT_HEADER, "caller-project", 2, "configured-project"),
        (
            "openai-project",
            "lower-case-project",
            3,
            "lower-case-project",
        ),
    ];
    for (name, caller, length, kept) in cases {
        let mut headers = support::headers();
        headers.insert(name.to_owned(), caller.to_owned());
        let params = EnvironmentParams {
            api_key: None,
            project_id: Some("configured-project"),
        };
        validate_environment(&mut headers, params, &globals);
        assert_eq!(support::header(&headers, name).as_deref(), Some(kept));
        assert_eq!(
            support::header(&headers, PROJECT_HEADER).as_deref(),
            Some("configured-project")
        );
        assert_eq!(headers.len(), length);
    }
}

fn unchanged_cases() {
    let globals = support::globals();
    let mut headers = support::headers();
    headers.insert(PROJECT_HEADER.to_owned(), "caller-project".to_owned());
    let params = EnvironmentParams {
        api_key: None,
        project_id: None,
    };
    validate_environment(&mut headers, params, &globals);
    assert_eq!(
        support::header(&headers, PROJECT_HEADER).as_deref(),
        Some("caller-project")
    );
    let empty = EnvironmentParams {
        api_key: None,
        project_id: Some(""),
    };
    validate_environment(&mut headers, empty, &globals);
    assert_eq!(
        support::header(&headers, PROJECT_HEADER).as_deref(),
        Some("caller-project")
    );
    let mut absent = support::headers();
    validate_environment(&mut absent, empty, &globals);
    assert_eq!(absent.len(), 1);
}
