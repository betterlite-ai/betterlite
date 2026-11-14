#![cfg(test)]

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::case::HelperCase;
use crate::case::helper::validate_helper;
use crate::helpers::Helper as _;

fn call_input(file: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("cases/helpers/studio-file-search")
        .join(file);
    let case: HelperCase = serde_saphyr::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    case.call.input
}

fn run_case(file: &str) -> Value {
    super::Op::decode_and_run(&call_input(file)).unwrap()
}

#[test]
fn helper_seeds_select_studio_file_search() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/helper_case");
    let seeds: Vec<String> = fs::read_dir(corpus)
        .expect("helper corpus")
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("studio_file_search"))
        })
        .map(|path| fs::read_to_string(path).expect("seed file"))
        .collect();
    assert!(!seeds.is_empty(), "no studio_file_search seed exists");
    assert!(seeds.iter().all(|source| {
        let case: HelperCase = serde_saphyr::from_str(source).expect("helper seed");
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && case.call.operation == "studio_file_search"
            && super::Op::check(&case.call.input).is_ok()
    }));
}

#[test]
fn an_unknown_function_is_not_an_input() {
    let input = json!({"calls": [{"function": "nope"}]});
    assert!(super::Op::check(&input).is_err());
    let unknown_field = json!({"calls": [{"function": "url", "api_basex": "x"}]});
    assert!(super::Op::check(&unknown_field).is_err());
}

#[test]
fn oracle_capabilities_publish_the_native_endpoints() {
    let result = run_case("no-params-auth.yaml");
    assert_eq!(
        result["calls"][0],
        json!({
            "endpoints": {
                "read": [["POST", "/models/{model}:generateContent"]],
                "write": [["POST", "/fileSearchStores"]],
            },
            "credentials": {},
            "supported": ["max_num_results", "filters"],
        })
    );
}

#[test]
fn oracle_environment_never_resolves_a_key_without_params() {
    let result = run_case("no-params-auth.yaml");
    assert_eq!(
        result["calls"][1],
        json!({"headers": {
            "Content-Type": "application/custom",
            "X-Custom": "kept",
            "x-goog-api-key": "caller-key",
        }})
    );
    assert_eq!(
        result["calls"][2],
        json!({"headers": {"Content-Type": "application/json"}})
    );
}

#[test]
fn oracle_missing_search_key_is_a_value_error() {
    let error = super::Op::decode_and_run(&call_input("missing-search-key.yaml")).unwrap_err();
    assert_eq!(error.class, "ValueError");
    assert_eq!(error.status, None);
}

#[test]
fn oracle_base_keeps_trailing_slashes() {
    let result = run_case("untrimmed-base.yaml");
    assert_eq!(result["calls"][0]["url"], "https://cfg.example///v1beta");
    assert_eq!(result["calls"][1]["url"], "/v1beta");
    assert_eq!(
        result["calls"][2]["url"],
        "https://generativelanguage.googleapis.com/v1beta"
    );
    assert_eq!(
        result["calls"][3]["url"],
        "https://search.example///v1beta/models/gemini-2.5-flash:generateContent"
    );
    assert_eq!(
        result["calls"][4]["url"],
        "https://h/v1beta/models/gemini-2.5-pro:generateContent"
    );
    assert_eq!(
        result["calls"][5]["url"],
        "https://h/v1beta/models/openai/gemini-pro:generateContent"
    );
    assert_eq!(
        result["calls"][6]["url"],
        "https://h/v1beta/models/gemini-2.5-flash:generateContent"
    );
    assert_eq!(
        result["calls"][3]["body"],
        json!({
            "contents": [{"parts": [{"text": "q"}]}],
            "tools": [{"file_search": {"file_search_store_names": ["fileSearchStores/store-1"]}}],
        })
    );
}

fn metadata_filter(body: &Value) -> &str {
    body["tools"][0]["file_search"]["metadata_filter"]
        .as_str()
        .expect("the filter is a string")
}

#[test]
fn oracle_filter_scalars_keep_python_spelling() {
    let result = run_case("filter-scalar-spelling.yaml");
    assert_eq!(
        metadata_filter(&result["calls"][0]["body"]),
        "genre = \"drama\" AND missing = None AND score = 2.5 AND verified = True AND year = 2024 AND zero = 0"
    );
    assert_eq!(
        result["calls"][1]["body"]["tools"][0]["file_search"].get("metadata_filter"),
        None
    );
    assert_eq!(
        metadata_filter(&result["calls"][2]["body"]),
        "title = \"say \"hi\"\" AND whole = 1.0"
    );
}

#[test]
fn oracle_result_limit_requests_one_candidate() {
    let result = run_case("result-limit-one.yaml");
    assert_eq!(result["calls"][0]["body"].get("generationConfig"), None);
    assert_eq!(
        result["calls"][1]["body"]["generationConfig"],
        json!({"candidateCount": 1})
    );
    assert_eq!(
        result["calls"][2]["body"]["generationConfig"],
        json!({"candidateCount": 1})
    );
    assert_eq!(
        result["calls"][3]["body"]["generationConfig"],
        json!({"candidateCount": 1})
    );
    assert_eq!(result["calls"][4]["body"].get("generationConfig"), None);
}

#[test]
fn oracle_grounding_dedupes_supports() {
    let result = run_case("grounding-and-support-dedup.yaml");
    assert_eq!(
        result["calls"][0],
        json!({
            "object": "vector_store.search_results.page",
            "search_query": "seed query",
            "data": [
                {
                    "score": null,
                    "content": [{"text": "first passage", "type": "text"}],
                    "file_id": "files/a",
                    "filename": "a.txt",
                    "attributes": {"uri": "files/a", "title": "a.txt"},
                },
                {
                    "score": 0.5,
                    "content": [{"text": "answer one", "type": "text"}],
                    "attributes": {"grounding_chunk_indices": [0]},
                },
                {
                    "score": null,
                    "content": [{"text": "second passage", "type": "text"}],
                    "file_id": "files/b",
                    "filename": "b.txt",
                    "attributes": {"uri": "files/b", "title": "b.txt"},
                },
                {
                    "score": 0.25,
                    "content": [{"text": "answer two", "type": "text"}],
                    "attributes": {"grounding_chunk_indices": [0, 1]},
                },
            ],
        })
    );
}

#[test]
fn oracle_empty_retrieved_text_keeps_identifiers() {
    let result = run_case("empty-retrieved-text.yaml");
    assert_eq!(
        result["calls"][0],
        json!({
            "object": "vector_store.search_results.page",
            "search_query": "",
            "data": [
                {
                    "score": null,
                    "content": [{"text": "", "type": "text"}],
                    "file_id": "files/c",
                    "filename": null,
                    "attributes": {"uri": "files/c", "title": ""},
                },
                {
                    "score": null,
                    "content": [{"text": "", "type": "text"}],
                    "file_id": null,
                    "filename": null,
                    "attributes": {"uri": "", "title": ""},
                },
                {
                    "score": null,
                    "content": [{"text": "kept passage", "type": "text"}],
                    "file_id": null,
                    "filename": "named.txt",
                    "attributes": {"uri": "", "title": "named.txt"},
                },
                {
                    "score": null,
                    "content": [{"text": "kept", "type": "text"}],
                    "attributes": {"grounding_chunk_indices": []},
                },
            ],
        })
    );
}

#[test]
fn oracle_create_keeps_the_complete_store_id() {
    let result = run_case("complete-store-id.yaml");
    assert_eq!(
        result["calls"][0],
        json!({
            "url": "https://studio.example///v1beta/fileSearchStores",
            "body": {"displayName": "Docs"},
        })
    );
    assert_eq!(result["calls"][1]["body"], json!({}));
    assert_eq!(result["calls"][2]["body"], json!({}));
    assert_eq!(
        result["calls"][3],
        json!({
            "id": "fileSearchStores/abc",
            "object": "vector_store",
            "created_at": 1_704_164_645_i64,
            "name": "Docs",
            "bytes": 0,
            "file_counts": {
                "in_progress": 0,
                "completed": 0,
                "failed": 0,
                "cancelled": 0,
                "total": 0,
            },
            "status": "completed",
            "expires_after": null,
            "expires_at": null,
            "last_active_at": null,
            "metadata": null,
        })
    );
    assert_eq!(result["calls"][4]["id"], "fileSearchStores/xyz");
    assert_eq!(result["calls"][4]["created_at"], 1_704_144_845_i64);
    assert_eq!(result["calls"][4]["name"], "");
    assert_eq!(result["calls"][5]["id"], "");
    assert_eq!(result["calls"][5]["created_at"], 0);
    assert_eq!(result["calls"][6]["name"], "");
    assert_eq!(result["calls"][6]["created_at"], 0);
}

#[test]
fn oracle_malformed_create_reports_the_status() {
    let error = super::Op::decode_and_run(&call_input("malformed-create.yaml")).unwrap_err();
    assert_eq!(error.class, "GeminiError");
    assert_eq!(error.status, Some(422));
}
