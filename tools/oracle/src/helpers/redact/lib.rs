#![expect(
    clippy::absolute_paths,
    clippy::disallowed_methods,
    clippy::disallowed_types,
    clippy::pub_use,
    reason = "The operation files name crate::helpers paths, build provider JSON with serde_json::json!, and reach the trait through the re-export the shared macro generates."
)]

betterlite_oracle_helper_api::helper_group! {
    "redaction" => redaction,
    "sentry_scrubbing" => sentry_scrubbing,
    "structured_redaction" => structured_redaction,
}
