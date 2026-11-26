use super::super::base_url::{VertexLocationError, passthrough_vertex_base_url};

#[test]
fn the_global_location_uses_the_global_host() {
    assert_eq!(
        passthrough_vertex_base_url(Some("global")).unwrap(),
        "https://aiplatform.googleapis.com/"
    );
}

#[test]
fn a_regional_location_uses_the_location_host() {
    assert_eq!(
        passthrough_vertex_base_url(Some("us-central1")).unwrap(),
        "https://us-central1-aiplatform.googleapis.com/"
    );
    assert_eq!(
        passthrough_vertex_base_url(Some("europe-west4")).unwrap(),
        "https://europe-west4-aiplatform.googleapis.com/"
    );
}

#[test]
fn a_single_word_location_uses_the_rep_host() {
    assert_eq!(
        passthrough_vertex_base_url(Some("us")).unwrap(),
        "https://aiplatform.us.rep.googleapis.com/"
    );
}

#[test]
fn a_missing_location_is_required() {
    let error = passthrough_vertex_base_url(None).unwrap_err();
    assert_eq!(error, VertexLocationError::Required);
    assert_eq!(error.to_string(), "vertex_location is required");
}

#[test]
fn an_uppercase_location_is_an_invalid_format() {
    let error = passthrough_vertex_base_url(Some("US-Central1")).unwrap_err();
    assert_eq!(error, VertexLocationError::InvalidFormat);
    assert_eq!(error.to_string(), "Invalid vertex_location format");
}

#[test]
fn a_trailing_newline_is_an_invalid_format() {
    assert_eq!(
        passthrough_vertex_base_url(Some("us-central1\n")).unwrap_err(),
        VertexLocationError::InvalidFormat
    );
    assert_eq!(
        passthrough_vertex_base_url(Some("global\n")).unwrap_err(),
        VertexLocationError::InvalidFormat
    );
}

#[test]
fn an_empty_or_symbol_location_is_an_invalid_format() {
    assert_eq!(
        passthrough_vertex_base_url(Some("")).unwrap_err(),
        VertexLocationError::InvalidFormat
    );
    assert_eq!(
        passthrough_vertex_base_url(Some("1central")).unwrap_err(),
        VertexLocationError::InvalidFormat
    );
    assert_eq!(
        passthrough_vertex_base_url(Some("-us")).unwrap_err(),
        VertexLocationError::InvalidFormat
    );
    assert_eq!(
        passthrough_vertex_base_url(Some("us_central1")).unwrap_err(),
        VertexLocationError::InvalidFormat
    );
    assert_eq!(
        passthrough_vertex_base_url(Some("us central1")).unwrap_err(),
        VertexLocationError::InvalidFormat
    );
    assert_eq!(
        passthrough_vertex_base_url(Some("us/../x")).unwrap_err(),
        VertexLocationError::InvalidFormat
    );
}
