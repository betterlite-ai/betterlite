use betterlite_prompts::arguments::json::Object;

use crate::api::messages_content_primitives::tools::TRANSLATABLE_ANTHROPIC_PARAMS;

// Only truthy stop_sequences becomes the chat stop list.
pub fn translate_stop_sequences(request: &Object, kwargs: &mut Object) {
    let Some(stop_sequences) = request
        .get("stop_sequences")
        .filter(|value| value.is_truthy())
    else {
        return;
    };
    kwargs.insert("stop".to_owned(), stop_sequences.clone());
}

// The final copy runs last, so a request key sharing a translated name wins over the translated value.
pub fn copy_untranslated_params(request: &Object, kwargs: &mut Object) {
    for (key, value) in request {
        if !TRANSLATABLE_ANTHROPIC_PARAMS.contains(&key.as_str()) {
            kwargs.insert(key.clone(), value.clone());
        }
    }
}
