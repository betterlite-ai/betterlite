use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;
mod tests;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FileType {
    Aac,
    Csv,
    Doc,
    Docx,
    Flac,
    Flv,
    Gif,
    GoogleDoc,
    GoogleDrawings,
    GoogleSheets,
    GoogleSlides,
    Heic,
    Heif,
    Html,
    Jpeg,
    Json,
    M4a,
    M4v,
    Mov,
    Mp3,
    Mp4,
    Mpeg,
    Mpegps,
    Mpg,
    Mpa,
    Mpga,
    Ogg,
    Opus,
    Pdf,
    Pcm,
    Png,
    Ppt,
    Pptx,
    Rtf,
    #[serde(rename = "3GPP")]
    ThreeGpp,
    Txt,
    Wav,
    Webm,
    Webp,
    Wmv,
    Xls,
    Xlsx,
}
#[rustfmt::skip]
const EXTENSIONS: &[(FileType, &[&str])] = &[ (FileType::Aac, &["aac"]), (FileType::Csv, &["csv"]), (FileType::Doc, &["doc"]), (FileType::Docx, &["docx"]), (FileType::Flac, &["flac"]), (FileType::Flv, &["flv"]), (FileType::Gif, &["gif"]), (FileType::GoogleDoc, &["gdoc"]), (FileType::GoogleDrawings, &["gdraw"]), (FileType::GoogleSheets, &["gsheet"]), (FileType::GoogleSlides, &["gslides"]), (FileType::Heic, &["heic"]), (FileType::Heif, &["heif"]), (FileType::Html, &["html", "htm"]), (FileType::Jpeg, &["jpeg", "jpg"]), (FileType::Json, &["json"]), (FileType::M4a, &["m4a"]), (FileType::M4v, &["m4v"]), (FileType::Mov, &["mov"]), (FileType::Mp3, &["mp3"]), (FileType::Mp4, &["mp4"]), (FileType::Mpeg, &["mpeg"]), (FileType::Mpegps, &["mpegps"]), (FileType::Mpg, &["mpg"]), (FileType::Mpa, &["mpa"]), (FileType::Mpga, &["mpga"]), (FileType::Ogg, &["ogg"]), (FileType::Opus, &["opus"]), (FileType::Pdf, &["pdf"]), (FileType::Pcm, &["pcm"]), (FileType::Png, &["png"]), (FileType::Ppt, &["ppt"]), (FileType::Pptx, &["pptx"]), (FileType::Rtf, &["rtf"]), (FileType::ThreeGpp, &["3gpp"]), (FileType::Txt, &["txt"]), (FileType::Wav, &["wav"]), (FileType::Webm, &["webm"]), (FileType::Webp, &["webp"]), (FileType::Wmv, &["wmv"]), (FileType::Xls, &["xls"]), (FileType::Xlsx, &["xlsx"]),
];
#[rustfmt::skip]
const MIME_TYPES: &[(FileType, &str)] = &[ (FileType::Aac, "audio/aac"), (FileType::Csv, "text/csv"), (FileType::Doc, "application/msword"),
    (
        FileType::Docx,
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    ), (FileType::Flac, "audio/flac"), (FileType::Flv, "video/x-flv"), (FileType::Gif, "image/gif"), (FileType::GoogleDoc, "application/vnd.google-apps.document"),
    (
        FileType::GoogleDrawings,
        "application/vnd.google-apps.drawing",
    ),
    (
        FileType::GoogleSheets,
        "application/vnd.google-apps.spreadsheet",
    ),
    (
        FileType::GoogleSlides,
        "application/vnd.google-apps.presentation",
    ), (FileType::Heic, "image/heic"), (FileType::Heif, "image/heif"), (FileType::Html, "text/html"), (FileType::Jpeg, "image/jpeg"), (FileType::Json, "application/json"), (FileType::M4a, "audio/x-m4a"), (FileType::M4v, "video/x-m4v"), (FileType::Mov, "video/quicktime"), (FileType::Mp3, "audio/mpeg"), (FileType::Mp4, "video/mp4"), (FileType::Mpeg, "video/mpeg"), (FileType::Mpegps, "video/mpegps"), (FileType::Mpg, "video/mpg"), (FileType::Mpa, "audio/m4a"), (FileType::Mpga, "audio/mpga"), (FileType::Ogg, "audio/ogg"), (FileType::Opus, "audio/opus"), (FileType::Pdf, "application/pdf"), (FileType::Pcm, "audio/pcm"), (FileType::Png, "image/png"), (FileType::Ppt, "application/vnd.ms-powerpoint"),
    (
        FileType::Pptx,
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    ), (FileType::Rtf, "application/rtf"), (FileType::ThreeGpp, "video/3gpp"), (FileType::Txt, "text/plain"), (FileType::Wav, "audio/wav"), (FileType::Webm, "video/webm"), (FileType::Webp, "image/webp"), (FileType::Wmv, "video/wmv"), (FileType::Xls, "application/vnd.ms-excel"),
    (
        FileType::Xlsx,
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    ),
];
#[derive(Debug, Error, PartialEq, Eq)]
pub enum FileTypeError {
    #[error("Unknown file type for extension: {0}")]
    UnknownExtension(String),
    #[error("Unknown extension for mime type: {0}")]
    UnknownMime(String),
}
fn find_type(extension: &str) -> Option<FileType> {
    let lower = extension.to_lowercase();
    EXTENSIONS.iter().find_map(|(file_type, values)| {
        values
            .iter()
            .any(|value| *value == lower)
            .then_some(*file_type)
    })
}
pub fn type_from_extension(extension: &str) -> Result<FileType, FileTypeError> {
    let file_type = find_type(extension);
    file_type.ok_or_else(|| FileTypeError::UnknownExtension(extension.to_owned()))
}
#[must_use]
pub fn extension_from_type(file_type: FileType) -> &'static str {
    if let Some(extension) = EXTENSIONS.iter().find_map(|(kind, values)| {
        (*kind == file_type).then(|| values.first().copied().unwrap_or(""))
    }) {
        return extension;
    }
    ""
}
#[must_use]
pub fn mime_from_type(file_type: FileType) -> &'static str {
    for (kind, mime) in MIME_TYPES {
        if *kind == file_type {
            return mime;
        }
    }
    ""
}
pub fn mime_from_extension(extension: &str) -> Result<&'static str, FileTypeError> {
    let result = type_from_extension(extension).map(mime_from_type);
    result
}
pub fn extension_from_mime(mime_type: &str) -> Result<&'static str, FileTypeError> {
    let lower = mime_type.to_lowercase();
    MIME_TYPES
        .iter()
        .find_map(|(file_type, mime)| (*mime == lower).then(|| extension_from_type(*file_type)))
        .ok_or_else(|| FileTypeError::UnknownMime(mime_type.to_owned()))
}
const GEMINI_FILE_TYPES: &[FileType] = &[
    FileType::Png,
    FileType::Jpeg,
    FileType::Webp,
    FileType::Aac,
    FileType::Flac,
    FileType::Mp3,
    FileType::Mpa,
    FileType::Mpeg,
    FileType::Mpga,
    FileType::Opus,
    FileType::Pcm,
    FileType::Wav,
    FileType::Webm,
    FileType::Flv,
    FileType::Mov,
    FileType::Mpegps,
    FileType::Mpg,
    FileType::Mp4,
    FileType::Wmv,
    FileType::ThreeGpp,
    FileType::Pdf,
    FileType::Txt,
];
#[must_use]
pub fn gemini_accepts(file_type: FileType) -> bool {
    for candidate in GEMINI_FILE_TYPES {
        if *candidate == file_type {
            return true;
        }
    }
    false
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileContentProvider {
    Openai,
    Azure,
    VertexAi,
    Bedrock,
    HostedVllm,
    LitellmProxy,
    Anthropic,
    Manus,
    Mistral,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderUploadPayload {
    pub provider: FileContentProvider,
    pub value: ProviderUploadValue,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderUploadValue {
    pub filename: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UploadData {
    Text(String),
    Bytes(Vec<u8>),
    Provider(ProviderUploadPayload),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TwoStepFileUploadRequest {
    pub method: String,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub data: UploadData,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TwoStepFileUploadConfig {
    pub initial_request: TwoStepFileUploadRequest,
    pub upload_request: TwoStepFileUploadRequest,
    pub upload_url_location: UploadUrlLocation,
    pub upload_url_key: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UploadUrlLocation {
    Headers,
    Body,
}
pub type ByteStream = Box<dyn Iterator<Item = Vec<u8>> + Send>;
pub struct FileContentStreamingResult {
    pub stream_iterator: ByteStream,
    pub headers: BTreeMap<String, String>,
}
impl fmt::Debug for FileContentStreamingResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileContentStreamingResult")
            .field("headers", &self.headers)
            .finish_non_exhaustive()
    }
}
pub struct StreamingMediaUploadConfig {
    pub body_stream: ByteStream,
    pub content_type: Option<String>,
}
