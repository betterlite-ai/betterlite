use crate::stream_chunk_shaping::records::{StreamChunk, StreamDelta};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum SpecialDeltaAttribute {
    Audio,
    Images,
}

#[must_use]
pub fn has_special_delta_content(model_response: &StreamChunk) -> bool {
    let Some(choice) = model_response.choices.first() else {
        return false;
    };
    let delta = &choice.delta;
    delta.tool_calls.is_some()
        || delta.function_call.is_some()
        || delta.audio.is_some()
        || delta.images.is_some()
}

#[must_use]
pub const fn has_special_delta_attribute(
    delta: &StreamDelta,
    attribute: SpecialDeltaAttribute,
) -> bool {
    match attribute {
        SpecialDeltaAttribute::Audio => delta.audio.is_some(),
        SpecialDeltaAttribute::Images => delta.images.is_some(),
    }
}

pub fn copy_delta_attribute(
    source: &StreamDelta,
    target: &mut StreamDelta,
    attribute: SpecialDeltaAttribute,
) {
    match attribute {
        SpecialDeltaAttribute::Audio => target.audio.clone_from(&source.audio),
        SpecialDeltaAttribute::Images => target.images.clone_from(&source.images),
    }
}

#[must_use]
pub fn has_any_special_attributes(delta: &StreamDelta) -> bool {
    [SpecialDeltaAttribute::Audio, SpecialDeltaAttribute::Images]
        .into_iter()
        .any(|attribute| has_special_delta_attribute(delta, attribute))
}

pub fn apply_special_delta_attributes(delta: &StreamDelta, model_response: &mut StreamChunk) {
    let Some(choice) = model_response.choices.first_mut() else {
        return;
    };
    for attribute in [SpecialDeltaAttribute::Audio, SpecialDeltaAttribute::Images] {
        if has_special_delta_attribute(delta, attribute) {
            copy_delta_attribute(delta, &mut choice.delta, attribute);
        }
    }
}
