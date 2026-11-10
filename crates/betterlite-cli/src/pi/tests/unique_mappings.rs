fn listing_failure(text: &str) -> PiSyncError {
    let get = ScriptedGet::new(vec![body(200, text)]);
    fetch_model_listing(BASE_URL, &api_key(), &get, &[]).unwrap_err()
}

#[test]
fn unique_mappings_rejects_two_source_models_for_one_id() {
    let error = listing_failure(
        r#"{"data":[{"id":"a","source_model":"x"},{"id":"a","source_model":"y"}]}"#,
    );
    assert_eq!(error.kind(), Some(ListingFailure::BadBody));
    assert!(matches!(
        error,
        PiSyncError::ListingInvalid {
            violation: ListingViolation::DuplicateMappings
        }
    ));
}

#[test]
fn unique_mappings_rejects_a_missing_source_model_mapped_elsewhere() {
    // The first entry maps "a" to itself; the second maps it to "b"
    let error = listing_failure(r#"{"data":[{"id":"a"},{"id":"a","source_model":"b"}]}"#);
    assert!(matches!(
        error,
        PiSyncError::ListingInvalid {
            violation: ListingViolation::DuplicateMappings
        }
    ));
}

#[test]
fn unique_mappings_treats_a_missing_source_model_as_the_id() {
    let get = ScriptedGet::new(vec![body(
        200,
        r#"{"data":[{"id":"a"},{"id":"a","source_model":"a"}]}"#,
    )]);
    let models = fetch_model_listing(BASE_URL, &api_key(), &get, &[]).unwrap();
    assert_eq!(models.len(), 2);
}

#[test]
fn unique_mappings_allows_one_source_model_for_different_ids() {
    let get = ScriptedGet::new(vec![body(
        200,
        r#"{"data":[{"id":"a","source_model":"x"},{"id":"b","source_model":"x"}]}"#,
    )]);
    let models = fetch_model_listing(BASE_URL, &api_key(), &get, &[]).unwrap();
    assert_eq!(models.len(), 2);
}

#[test]
fn unique_mappings_dedupes_identical_entries_before_counting() {
    let get = ScriptedGet::new(vec![body(
        200,
        r#"{"data":[{"id":"a","source_model":"x"},{"id":"a","source_model":"x"}]}"#,
    )]);
    let models = fetch_model_listing(BASE_URL, &api_key(), &get, &[]).unwrap();
    assert_eq!(
        models,
        vec![ListedModel {
            id: "a".to_owned(),
            source_model: Some("x".to_owned())
        }]
    );
}
