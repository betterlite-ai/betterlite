use betterlite_vertex::gemini_media::equivalence::part_exists;
use betterlite_vertex::gemini_media::error::{MediaError, MediaField};
use betterlite_vertex::gemini_media::mime::apply_mime_alias;
use betterlite_vertex::gemini_media::process::{MediaRequest, process_media};
use betterlite_vertex::gemini_media::resolution::{convert_detail, max_media_resolution};
use serde_json::json;

use super::input::{EquivalenceCase, Input, MediaCase};
use crate::helpers::{HelperError, HelperResult};

pub(super) fn run(input: &Input) -> HelperResult {
    match input {
        Input::MimeAlias { values } => {
            let aliases: Vec<String> = values.iter().map(|value| apply_mime_alias(value)).collect();
            Ok(json!({"values": aliases}))
        }
        Input::ExplicitGcsFormat { cases }
        | Input::FilesUriWithoutMime { cases }
        | Input::HttpsPassthrough { cases }
        | Input::VideoOffsets { cases } => parts(cases),
        Input::ResolutionMaximum { messages } => Ok(json!({
            "maximum": max_media_resolution(messages),
        })),
        Input::PartKeyEquivalence { cases } => Ok(json!({"matches": matches(cases)})),
    }
}

fn parts(cases: &[MediaCase]) -> HelperResult {
    let mut output = Vec::with_capacity(cases.len());
    for case in cases {
        let request = MediaRequest {
            url: &case.url,
            hints: &case.hints,
            resolution: case.detail.as_deref().and_then(convert_detail),
            video_metadata: case.video_metadata.as_ref(),
            model: case.model.as_deref(),
            field: MediaField::ImageUrl,
        };
        output.push(process_media(&request).map_err(|error| media_error(&error))?);
    }
    Ok(json!({"parts": output}))
}

fn matches(cases: &[EquivalenceCase]) -> Vec<bool> {
    cases
        .iter()
        .map(|case| {
            let excluded: Vec<&str> = case.excluded_keys.iter().map(String::as_str).collect();
            part_exists(&case.parts, &case.part, &excluded)
        })
        .collect()
}

fn media_error(error: &MediaError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "BadRequestError".to_owned(),
        status: Some(error.failure().status),
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
