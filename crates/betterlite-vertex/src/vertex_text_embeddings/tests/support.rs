use betterlite_models::decisions::Extra;
use betterlite_models::decisions::Json;
use betterlite_openai::embedding::EmbeddingInput;
use betterlite_openai::embedding::EmbeddingVector;

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

pub(super) fn object(text: &str) -> Extra {
    let Json::Object(fields) = json(text) else {
        panic!("an options fixture must be an object");
    };
    fields
}

pub(super) fn text(value: &str) -> EmbeddingInput {
    EmbeddingInput::Text(value.to_owned())
}

pub(super) fn vector(text: &str) -> EmbeddingVector {
    let Json::List(items) = json(text) else {
        panic!("a vector fixture must be a list");
    };
    let values = items
        .into_iter()
        .map(|item| match item {
            Json::Number(number) => number,
            Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => {
                panic!("a vector fixture must hold numbers");
            }
        })
        .collect();
    EmbeddingVector::Floats(values)
}
