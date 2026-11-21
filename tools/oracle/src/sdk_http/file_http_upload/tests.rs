#![cfg(test)]

use std::collections::BTreeMap;

use indexmap::IndexMap;

use crate::sdk_http::OracleLimits;
use crate::sdk_http::call_kind::CallKind as _;

use super::{Call, Content, FileSpec, Plan, RunMode, UploadMethod};

fn uploading(also: Vec<Call>) -> Call {
    Call {
        mode: RunMode::Sync,
        upload_method: UploadMethod::default(),
        headers: BTreeMap::new(),
        litellm_params: IndexMap::new(),
        file: FileSpec {
            filename: None,
            content_type: None,
            content: Content::Text(String::from("x")),
        },
        timeout_seconds: None,
        plan: Plan::Text(String::from("x")),
        also,
    }
}

#[test]
fn validate_refuses_a_nested_extra_upload() {
    let flat = uploading(vec![uploading(Vec::new())]);
    flat.validate(&OracleLimits::default())
        .expect("one level of extra uploads is valid");
    let nested = uploading(vec![uploading(vec![uploading(Vec::new())])]);
    let error = nested.validate(&OracleLimits::default()).unwrap_err();
    assert_eq!(
        error.to_string(),
        "invalid sdk_http case: a file upload case runs one level of extra uploads"
    );
}
