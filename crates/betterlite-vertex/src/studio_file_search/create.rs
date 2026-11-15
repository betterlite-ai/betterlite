use betterlite_models::decisions::Json;
use indexmap::IndexMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatePlan {
    pub url: String,
    pub body: Json,
}

// transform_create_vector_store_request writes displayName only for a name it can see.
#[must_use]
pub fn create_request(name: Option<&str>, api_base: &str) -> CreatePlan {
    let mut body = IndexMap::new();
    if let Some(display) = name.filter(|candidate| !candidate.is_empty()) {
        body.insert("displayName".to_owned(), Json::Text(display.to_owned()));
    }
    CreatePlan {
        url: format!("{api_base}/fileSearchStores"),
        body: Json::Object(body),
    }
}
