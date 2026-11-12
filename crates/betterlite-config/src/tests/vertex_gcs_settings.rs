#![cfg(test)]

use std::collections::BTreeMap;

use crate::sections::vertex_gcs_settings::{GCS_BATCH_BUCKET_ENV, GCS_BUCKET_ENV};

#[test]
fn loads_gcs_bucket_name() {
    let config = super::map_loader(BTreeMap::new())
        .load(
            "model_list:\n  - model_name: a\n    litellm_params:\n      gcs_bucket_name: bucket-a\n",
        )
        .unwrap();

    let settings = &config.sections.vertex_gcs_settings;
    assert_eq!(settings.deployments.len(), 1);
    assert_eq!(
        settings.deployments[0].gcs_bucket_name.as_deref(),
        Some("bucket-a")
    );
    assert_eq!(settings.deployments[0].bucket_name, None);
    assert_eq!(settings.batch_environment, None);
    assert_eq!(settings.environment, None);
}

#[test]
fn loads_bucket_name() {
    let config = super::map_loader(BTreeMap::new())
        .load("model_list:\n  - model_name: a\n    litellm_params:\n      bucket_name: bucket-b\n")
        .unwrap();

    let settings = &config.sections.vertex_gcs_settings;
    assert_eq!(settings.deployments.len(), 1);
    assert_eq!(settings.deployments[0].gcs_bucket_name, None);
    assert_eq!(
        settings.deployments[0].bucket_name.as_deref(),
        Some("bucket-b")
    );
}

#[test]
fn loads_gcs_batch_bucket_env() {
    let config = super::map_loader(BTreeMap::from([(
        GCS_BATCH_BUCKET_ENV.to_owned(),
        "batch-bucket".to_owned(),
    )]))
    .load("")
    .unwrap();

    let settings = &config.sections.vertex_gcs_settings;
    assert_eq!(settings.batch_environment.as_deref(), Some("batch-bucket"));
    assert_eq!(settings.environment, None);
}

#[test]
fn loads_gcs_bucket_env() {
    let config = super::map_loader(BTreeMap::from([(
        GCS_BUCKET_ENV.to_owned(),
        "env-bucket".to_owned(),
    )]))
    .load("")
    .unwrap();

    let settings = &config.sections.vertex_gcs_settings;
    assert_eq!(settings.batch_environment, None);
    assert_eq!(settings.environment.as_deref(), Some("env-bucket"));
}

#[test]
fn loads_all_four_bucket_settings_distinctly() {
    let config = super::map_loader(BTreeMap::from([
        (GCS_BATCH_BUCKET_ENV.to_owned(), "batch-bucket".to_owned()),
        (GCS_BUCKET_ENV.to_owned(), "env-bucket".to_owned()),
    ]))
    .load(
        "model_list:\n  - model_name: a\n    litellm_params:\n      gcs_bucket_name: bucket-a\n      bucket_name: bucket-b\n",
    )
    .unwrap();

    let settings = &config.sections.vertex_gcs_settings;
    assert_eq!(
        settings.deployments[0].gcs_bucket_name.as_deref(),
        Some("bucket-a")
    );
    assert_eq!(
        settings.deployments[0].bucket_name.as_deref(),
        Some("bucket-b")
    );
    assert_eq!(settings.batch_environment.as_deref(), Some("batch-bucket"));
    assert_eq!(settings.environment.as_deref(), Some("env-bucket"));
}
