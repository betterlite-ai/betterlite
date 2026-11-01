use crate::arguments::json::{Json, Object};

pub(super) fn file_json(content: &str, metadata: Object) -> Object {
    let mut entry = Object::new();
    entry.insert("content".to_owned(), Json::Text(content.to_owned()));
    entry.insert("metadata".to_owned(), Json::Object(metadata));
    entry
}

mod tests;
