#![cfg(test)]

mod fixture;
mod secrets;
mod selection;
mod signing;
mod sts;

// the DoD filters anchor these names at the tests module, so they live here rather than in a submodule
#[tokio::test]
async fn fixed_clock_botocore_signature() {
    signing::pinned_vectors().await;
}

#[tokio::test]
async fn web_identity_duration_3600() {
    sts::web_identity_duration().await;
}
