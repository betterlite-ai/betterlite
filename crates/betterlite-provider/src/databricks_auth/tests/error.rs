use std::collections::BTreeMap;

use crate::databricks_auth::error::DatabricksError;
use betterlite_openai::error::ProviderError;

#[test]
fn a_databricks_error_converts_to_the_gateway_error() {
    let error = DatabricksError {
        status_code: 418,
        message: "boom".to_owned(),
        headers: Some(BTreeMap::from([(
            "x-databricks".to_owned(),
            "1".to_owned(),
        )])),
    };
    let converted = ProviderError::from(error);
    assert_eq!(converted.status_code, 418);
    assert_eq!(converted.message, "boom");
    assert_eq!(
        converted.headers,
        Some(vec![("x-databricks".to_owned(), "1".to_owned())])
    );
}

#[test]
fn an_empty_header_map_becomes_no_headers() {
    let error = DatabricksError {
        status_code: 400,
        message: "boom".to_owned(),
        headers: Some(BTreeMap::new()),
    };
    assert_eq!(ProviderError::from(error).headers, None);
}
