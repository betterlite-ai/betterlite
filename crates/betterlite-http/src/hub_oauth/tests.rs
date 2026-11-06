#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::hub_oauth::stored_tokens::parse::parse;

mod clock;
mod lock;
mod oauth;
mod oidc;
mod parser;
mod support;

#[tokio::test]
async fn oidc_follows_huggingface_hub() {
    oidc::exchanges_the_env_id_token().await;
    oidc::caches_a_supplied_token_for_its_whole_lifetime().await;
    oidc::explicit_token_makes_no_request().await;
    oidc::mints_a_github_actions_token_first().await;
    oidc::renews_a_minted_token_before_it_ends().await;
    oidc::unset_resource_falls_through_to_the_env_token().await;
    oidc::missing_inputs_are_errors_even_with_an_env_token().await;
    oidc::rejected_exchanges_are_errors_and_not_cached().await;
    oidc::exchange_uses_the_download_timeout().await;
}

#[tokio::test]
async fn oauth_refresh_follows_huggingface_hub() {
    oauth::token_is_refreshed_a_day_before_it_ends().await;
    oauth::tokens_without_usable_metadata_are_returned_as_they_are().await;
    oauth::refresh_request_is_a_form_post().await;
    oauth::refreshed_token_is_saved_with_private_modes().await;
    oauth::refreshed_token_is_rechecked_after_five_minutes().await;
    oauth::failed_refresh_warns_once().await;
    oauth::invalid_grant_warns_and_is_never_rechecked().await;
    oauth::lock_timeout_keeps_the_stored_token().await;
    oauth::refresh_by_another_process_is_adopted().await;
    oauth::token_file_that_moved_meanwhile_is_not_overwritten().await;
    oauth::stored_tokens_that_are_not_utf8_are_an_error().await;
    oauth::unparsable_stored_tokens_count_as_empty().await;
    oauth::token_file_rewritten_during_the_lookup_is_served().await;
}

#[rstest]
fn replays_the_stored_tokens_corpus(#[files("fuzz/corpus/stored_tokens/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    if let Ok(file) = parse(&bytes) {
        for section in file.with_defaults() {
            let mut names: Vec<&String> =
                section.fields.iter().map(|(name, _value)| name).collect();
            let count = names.len();
            names.sort();
            names.dedup();
            assert_eq!(names.len(), count);
        }
    }
}
