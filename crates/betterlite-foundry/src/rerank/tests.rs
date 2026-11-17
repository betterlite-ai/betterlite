#![cfg(test)]

mod auth_cases;
mod invariants;
mod reply_cases;
mod request_cases;
mod support;
mod url_cases;

use rstest::rstest;

#[rstest]
#[case::rerank_url_api_version(0)]
#[case::query_documents_top_n(1)]
#[case::model_group_normalization(2)]
#[case::relevance_scores(3)]
#[case::return_documents(4)]
#[case::rerank_error(5)]
#[case::foundry_rerank_auth(6)]
fn rerank_cases(#[case] case: u8) {
    match case {
        0 => url_cases::rerank_url_api_version(),
        1 => request_cases::query_documents_top_n(),
        2 => reply_cases::model_group_normalization(),
        3 => reply_cases::relevance_scores(),
        4 => request_cases::return_documents(),
        5 => reply_cases::rerank_error(),
        _ => auth_cases::foundry_rerank_auth(),
    }
}

#[test]
fn foundry_rerank_credential_identity_isolation() {
    invariants::credential_identity_isolation();
}

#[test]
fn foundry_rerank_secret_safe_failure() {
    invariants::secret_safe_failure();
}
