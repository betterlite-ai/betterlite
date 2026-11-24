#![cfg(test)]

use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_vertex::studio_identity::AuthValue;

use super::gemini_key;

#[test]
fn gemini_key_reads_the_kwarg() {
    let extra = Object::from([("gemini_api_key".to_owned(), Json::Text("probe".to_owned()))]);
    let key = gemini_key(&extra).expect("the kwarg is a studio key");
    assert!(matches!(key, AuthValue::Key(_)));
    assert!(gemini_key(&Object::new()).is_none());
}
