use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use betterlite_vertex::gemini_media::equivalence::part_exists;
use betterlite_vertex::gemini_media::error::{MediaError, MediaField};
use betterlite_vertex::gemini_media::mime::{MimeHints, apply_mime_alias};
use betterlite_vertex::gemini_media::part::VideoMetadata;
use betterlite_vertex::gemini_media::process::{MediaRequest, process_media};
use betterlite_vertex::gemini_media::resolution::{convert_detail, max_media_resolution};
use serde::Deserialize;
use serde_json::json;
use crate::compare::helper::ErrorCode;
use crate::helpers::{HelperError, HelperResult};
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Call {
    MimeAlias,
    ExplicitGcsFormat,
    FilesUriWithoutMime,
    HttpsPassthrough,
    VideoOffsets,
    ResolutionMaximum,
    PartKeyEquivalence,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    call: Call,
    #[serde(default)]
    values: Vec<String>,
    #[serde(default)]
    cases: Vec<Case>,
    #[serde(default)]
    messages: Vec<Message>,
}
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Case {
    url: String,
    hints: MimeHints,
    detail: Option<String>,
    model: Option<String>,
    video_metadata: Option<VideoMetadata>,
    parts: Vec<Object>,
    part: Object,
    excluded_keys: Vec<String>,
}
pub(super) fn run(input: &Input) -> HelperResult {
    match input.call {
        Call::MimeAlias => {
            let aliases: Vec<String> = input.values.iter().map(|value| apply_mime_alias(value)).collect();
            Ok(json!({"values": aliases}))
        }
        Call::ExplicitGcsFormat | Call::FilesUriWithoutMime | Call::HttpsPassthrough | Call::VideoOffsets => {
            parts(&input.cases)
        }
        Call::ResolutionMaximum => Ok(json!({
            "maximum": max_media_resolution(&input.messages),
        })),
        Call::PartKeyEquivalence => Ok(json!({"matches": matches(&input.cases)})),
    }
}
fn parts(cases: &[Case]) -> HelperResult {
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
        output.push(process_media(&request).map_err(|error| {
            Box::new(HelperError {
                class: "BadRequestError".to_owned(),
                status: Some(error.status()),
                error_type: Some(Some(error.error_type().to_owned())),
                param: Some(error.param().map(str::to_owned)),
                code: error.code().map(|code| Some(ErrorCode::Text(code.to_owned()))),
                message: error.to_string(),
            })
        })?);
    }
    Ok(json!({"parts": output}))
}
fn matches(cases: &[Case]) -> Vec<bool> {
    cases
        .iter()
        .map(|case| {
            let excluded: Vec<&str> = case.excluded_keys.iter().map(String::as_str).collect();
            part_exists(&case.parts, &case.part, &excluded)
        })
        .collect()
}
