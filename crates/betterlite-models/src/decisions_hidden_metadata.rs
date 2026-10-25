#![cfg(test)]
use crate::decisions::types::{DecisionsResponse, ExtraFields};

#[test]
fn hidden_metadata_is_not_serialized() {
    let mut response: DecisionsResponse =
        serde_json::from_str(r#"{"answers":{},"usage":null}"#).expect("response");
    let mut hidden = ExtraFields::new();
    hidden.insert(
        "provider".to_owned(),
        serde_json::from_str(r#""systemone""#).expect("value"),
    );
    response.set_hidden_params(&hidden);
    assert_eq!(
        response
            .hidden_params()
            .get("provider")
            .map(ToString::to_string),
        Some(r#""systemone""#.to_owned())
    );
    assert!(
        !serde_json::to_value(response)
            .expect("serialize")
            .to_string()
            .contains("provider")
    );
}
