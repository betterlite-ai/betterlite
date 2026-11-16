use betterlite_models::decisions::Json;

use crate::fields::response::ServedTexts;

// LiteLLM's overlay_served_output_texts: each served text replaces that choice's message content.
#[must_use]
pub fn overlay_served_output_texts(response: &Json, served_texts: Option<&ServedTexts>) -> Json {
    let Some(texts) = served_texts else {
        return response.clone();
    };
    let Json::Object(fields) = response else {
        return response.clone();
    };
    let Some(Json::List(choices)) = fields.get("choices") else {
        return response.clone();
    };
    let overlaid: Vec<Json> = choices
        .iter()
        .enumerate()
        .map(|(index, choice)| match texts.get(index) {
            Some(Some(text)) => choice_with_text(choice, text),
            Some(None) | None => choice.clone(),
        })
        .collect();
    let mut logged = fields.clone();
    logged.insert("choices".to_owned(), Json::List(overlaid));
    Json::Object(logged)
}

fn choice_with_text(choice: &Json, text: &str) -> Json {
    let Json::Object(fields) = choice else {
        return choice.clone();
    };
    let Some(Json::Object(message)) = fields.get("message") else {
        return choice.clone();
    };
    if message.get("content") == Some(&Json::Text(text.to_owned())) {
        return choice.clone();
    }
    let mut replaced = fields.clone();
    let mut edited = message.clone();
    edited.insert("content".to_owned(), Json::Text(text.to_owned()));
    replaced.insert("message".to_owned(), Json::Object(edited));
    Json::Object(replaced)
}
