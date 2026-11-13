#![cfg(test)]

use std::fs;
use std::path::Path;

use rstest::rstest;

use crate::case::{HelperCase, helper::validate_helper};

#[test]
fn helper_seeds_select_xai_files() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus")
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("xai_files"))
        })
        .map(|path| fs::read_to_string(path).expect("seed file"))
        .collect();
    assert!(!seeds.is_empty());
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("helper seed");
        validate_helper(Path::new("<helper>"), &case).is_ok() && case.call.operation == "xai_files"
    }));
}

// the expected answers are LiteLLM's own, captured from the Python helper on the same case
#[rstest]
#[case::encoded_id_and_missing_content_id("encoded-id-and-missing-content-id.yaml", r#"["https://api.x.ai/v1/files/a%2Fb%20c","https://api.x.ai/v1/files/a%2Fb%20c/content",{"raised":null},{"raised":null},{"raised":null},{"raised":null},{"status":500,"headers":{"x-a":"b"},"body":[114,97,119]},{"status":206,"headers":{},"body":[]}]"#)]
#[case::forced_purpose_and_clock("forced-purpose-and-clock.yaml", r#"["Bearer module-key","Bearer explicit-key","https://env.example/v1/files",{"id":"file-a","bytes":0,"created_at":11,"filename":"","object":"file","purpose":"batch","status":"uploaded"},{"id":"file-b","bytes":0,"created_at":1700000900,"filename":"","object":"file","purpose":"batch","status":"uploaded"}]"#)]
#[case::list_stop_and_delete_default("list-stop-and-delete-default.yaml", r#"[["https://api.x.ai/v1/files",[["pagination_token","tok"]]],[{"id":"f1","bytes":0,"created_at":1700000000,"filename":"","object":"file","purpose":"batch","status":"uploaded"}],null,null,{"raised":null},["https://api.x.ai/v1/files",[["pagination_token","t"]]],{"raised":500},[],{"id":"file-1","deleted":true,"object":"file"},{"id":"file-2","deleted":false,"object":"file"},{"raised":null},{"raised":401}]"#)]
#[case::upload_defaults("upload-defaults.yaml", r#"["Bearer sk-key","https://api.x.ai/v1/files","https://proxy.example/v1/files",{"id":"file-1","bytes":0,"created_at":1700000000,"filename":"","object":"file","purpose":"batch","status":"uploaded"},{"id":"file-2","bytes":5,"created_at":12,"filename":"f","object":"file","purpose":"batch","status":"uploaded","expires_at":5},{"raised":null},{"raised":404}]"#)]
fn the_native_helper_answers_like_litellm(#[case] file: &str, #[case] litellm: &str) {
    use crate::helpers::Helper as _;

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/xai-files")
        .join(file);
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let answer = super::Op::decode_and_run(&case.call.input).unwrap();
    let expected: serde_json::Value = serde_json::from_str(litellm).unwrap();
    assert_eq!(answer, expected);
}
