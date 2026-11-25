#![cfg(test)]

use super::*;

// BaseLLMException's own defaults: the provider's status, no headers, and a POST request to LiteLLM's docs URL
#[test]
fn base_llm_exception_answers_the_status_with_a_post_request() {
    let response =
        ResponseFacts::base_llm_exception(Some(498)).expect("a status carries a response");
    assert_eq!(response.status_code, 498);
    assert!(response.headers.is_empty());
    assert_eq!(
        response.request,
        Some(RequestFacts {
            method: "POST".to_owned(),
            url: "https://docs.litellm.ai/docs".to_owned(),
        })
    );
}

#[test]
fn base_llm_exception_without_a_status_has_no_response() {
    assert_eq!(ResponseFacts::base_llm_exception(None), None);
}
