#![cfg(test)]

mod auth;
mod cases;
mod support;
mod urls;

use rstest::rstest;

#[rstest]
#[case::container_api_version_query(0)]
#[case::container_normalized_base(1)]
#[case::video_supported_fields(2)]
#[case::video_auth_and_url(3)]
#[case::vector_search_azure_base(4)]
fn resource_urls_cases(#[case] case: u8) {
    match case {
        0 => urls::container_api_version_query(),
        1 => urls::container_normalized_base(),
        2 => urls::video_supported_fields(),
        3 => auth::video_auth_and_url(),
        _ => urls::vector_search_azure_base(),
    }
}
