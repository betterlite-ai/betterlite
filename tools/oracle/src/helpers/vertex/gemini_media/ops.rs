use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::message_content::Message;
use betterlite_vertex::gemini_media::equivalence::part_exists;
use betterlite_vertex::gemini_media::error::MediaField;
use betterlite_vertex::gemini_media::mime::{MimeHints, apply_mime_alias};
use betterlite_vertex::gemini_media::part::VideoMetadata;
use betterlite_vertex::gemini_media::process::{MediaRequest, process_media};
use betterlite_vertex::gemini_media::resolution::{convert_detail, max_media_resolution};
use serde::Deserialize;
use serde_json::{Value, json};
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
pub(super) fn run(input: &Input) -> Value {
    match input.call {
        Call::MimeAlias => {
            let aliases: Vec<String> = input
                .values
                .iter()
                .map(|value| apply_mime_alias(value))
                .collect();
            json!({"values": aliases})
        }
        Call::ExplicitGcsFormat
        | Call::FilesUriWithoutMime
        | Call::HttpsPassthrough
        | Call::VideoOffsets => parts(&input.cases),
        Call::ResolutionMaximum => json!({
            "maximum": max_media_resolution(&input.messages),
        }),
        Call::PartKeyEquivalence => json!({
            "matches": input.cases.iter().map(|case| {
                let excluded: Vec<&str> = case.excluded_keys.iter().map(String::as_str).collect();
                part_exists(&case.parts, &case.part, &excluded)
            }).collect::<Vec<bool>>(),
        }),
    }
}
fn parts(cases: &[Case]) -> Value {
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
        let entry = match process_media(&request) {
            Ok(part) => json!(part),
            Err(error) => json!({"error": {
                "status": error.status(),
                "type": error.error_type(),
                "param": error.param(),
                "code": error.code(),
            }}),
        };
        output.push(entry);
    }
    json!({"parts": output})
}
