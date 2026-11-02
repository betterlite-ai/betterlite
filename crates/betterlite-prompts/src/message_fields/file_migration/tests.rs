#![cfg(test)]

use super::*;

fn file_part(file: Object) -> ContentPart {
    ContentPart {
        kind: "file".to_owned(),
        fields: Object::from_iter([("file".to_owned(), Json::Object(file))]),
    }
}

mod migrates_a_valid_file_reference;
mod rejects_an_unusable_file_reference;
