use std::fmt::{Display, Formatter, Result as FmtResult};

use base64::engine::{GeneralPurpose, GeneralPurposeConfig};
use base64::{Engine as _, alphabet};
use betterlite_prompts::message_content::{Content, ContentPart, Message};
use serde::{Deserialize, Serialize};

pub const MAX_FILE_SIZE_BYTES: usize = 4 * 1024 * 1024;

const REMOTE_URI_SCHEMES: [&str; 3] = ["gs://", "http://", "https://"];

// Python's b64decode(validate=True) requires canonical padding and rejects whitespace and foreign characters, but ignores non-zero trailing bits
const STRICT_BASE64: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_allow_trailing_bits(true),
);

const MIME_TO_BYTE_DATA_TYPE: [(&str, ByteDataType); 20] = [
    ("application/pdf", ByteDataType::Pdf),
    ("application/msword", ByteDataType::WordDocument),
    (
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        ByteDataType::WordDocument,
    ),
    (
        "application/vnd.openxmlformats-officedocument.wordprocessingml.template",
        ByteDataType::WordDocument,
    ),
    (
        "application/vnd.ms-word.document.macroenabled.12",
        ByteDataType::WordDocument,
    ),
    (
        "application/vnd.ms-word.template.macroenabled.12",
        ByteDataType::WordDocument,
    ),
    ("application/vnd.ms-excel", ByteDataType::ExcelDocument),
    (
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ByteDataType::ExcelDocument,
    ),
    (
        "application/vnd.openxmlformats-officedocument.spreadsheetml.template",
        ByteDataType::ExcelDocument,
    ),
    (
        "application/vnd.ms-excel.sheet.macroenabled.12",
        ByteDataType::ExcelDocument,
    ),
    (
        "application/vnd.ms-excel.template.macroenabled.12",
        ByteDataType::ExcelDocument,
    ),
    (
        "application/vnd.ms-powerpoint",
        ByteDataType::PowerpointDocument,
    ),
    (
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        ByteDataType::PowerpointDocument,
    ),
    (
        "application/vnd.openxmlformats-officedocument.presentationml.template",
        ByteDataType::PowerpointDocument,
    ),
    (
        "application/vnd.openxmlformats-officedocument.presentationml.slideshow",
        ByteDataType::PowerpointDocument,
    ),
    (
        "application/vnd.ms-powerpoint.presentation.macroenabled.12",
        ByteDataType::PowerpointDocument,
    ),
    (
        "application/vnd.ms-powerpoint.template.macroenabled.12",
        ByteDataType::PowerpointDocument,
    ),
    (
        "application/vnd.ms-powerpoint.slideshow.macroenabled.12",
        ByteDataType::PowerpointDocument,
    ),
    ("text/csv", ByteDataType::Csv),
    ("text/plain", ByteDataType::Txt),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ByteDataType {
    Pdf,
    WordDocument,
    ExcelDocument,
    PowerpointDocument,
    Csv,
    Txt,
}

impl ByteDataType {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pdf => "PDF",
            Self::WordDocument => "WORD_DOCUMENT",
            Self::ExcelDocument => "EXCEL_DOCUMENT",
            Self::PowerpointDocument => "POWERPOINT_DOCUMENT",
            Self::Csv => "CSV",
            Self::Txt => "TXT",
        }
    }
}

impl Display for ByteDataType {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileAttachment {
    pub file_bytes: Vec<u8>,
    pub byte_data_type: ByteDataType,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileScanPlan {
    pub attachments: Vec<FileAttachment>,
    pub unscannable_count: usize,
}

#[derive(Debug, Deserialize)]
struct FileBlock {
    file: FileData,
}

#[derive(Debug, Deserialize)]
struct FileData {
    file_data: Option<String>,
    format: Option<String>,
    filename: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DocumentBlock {
    source: DocumentSource,
}

#[derive(Debug, Deserialize)]
struct DocumentSource {
    data: Option<String>,
    media_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Block {
    File(FileBlock),
    Document(DocumentBlock),
}

enum Classified {
    Attachment(FileAttachment),
    Skip,
    Unscannable,
}

#[must_use]
pub fn plan_file_scans(messages: &[Message]) -> FileScanPlan {
    let mut plan = FileScanPlan::default();
    for message in messages {
        let Some(parts) = content_parts(message) else {
            continue;
        };
        for part in parts {
            match classify_block(part) {
                Classified::Attachment(attachment) => plan.attachments.push(attachment),
                Classified::Skip => {}
                Classified::Unscannable => {
                    plan.unscannable_count = plan.unscannable_count.saturating_add(1);
                }
            }
        }
    }
    plan
}

fn content_parts(message: &Message) -> Option<&[ContentPart]> {
    match &message.content {
        Some(Content::Parts(parts)) => Some(parts),
        Some(Content::Text(_)) | None => None,
    }
}

fn classify_block(part: &ContentPart) -> Classified {
    let Ok(bytes) = serde_json::to_vec(part) else {
        return Classified::Skip;
    };
    let Ok(block) = serde_json::from_slice::<Block>(&bytes) else {
        return Classified::Skip;
    };
    match block {
        Block::File(file) => classify_file(&file),
        Block::Document(document) => classify_document(&document),
    }
}

fn classify_file(block: &FileBlock) -> Classified {
    let Some(raw) = block
        .file
        .file_data
        .as_deref()
        .filter(|raw| !raw.is_empty())
    else {
        return Classified::Unscannable;
    };
    if is_remote_uri(raw) {
        return Classified::Unscannable;
    }
    let (uri_mime, parsed) = parse_data_uri(raw);
    let data = parsed.unwrap_or(raw);
    let declared = declared_byte_data_type(&block.file);
    let header = mime_to_byte_data_type(uri_mime);
    let Some(declared) = declared else {
        return decode_attachment(header, data);
    };
    if let Some(header) = header.filter(|header| *header != declared) {
        tracing::warn!(
            uri_mime = ?uri_mime,
            header = %header,
            declared = %declared,
            "Model Armor: the data URI MIME maps to a different byteDataType than the attachment declares; scanning with the declared type"
        );
    }
    decode_attachment(Some(declared), data)
}

fn classify_document(block: &DocumentBlock) -> Classified {
    let Some(data) = block.source.data.as_deref().filter(|data| !data.is_empty()) else {
        return Classified::Unscannable;
    };
    decode_attachment(
        mime_to_byte_data_type(block.source.media_type.as_deref()),
        data,
    )
}

fn declared_byte_data_type(file: &FileData) -> Option<ByteDataType> {
    let from_format = mime_to_byte_data_type(file.format.as_deref());
    from_format.or_else(|| {
        file.filename
            .as_deref()
            .and_then(|filename| mime_guess::from_path(filename).first_raw())
            .and_then(|mime| mime_to_byte_data_type(Some(mime)))
    })
}

fn decode_attachment(byte_data_type: Option<ByteDataType>, data: &str) -> Classified {
    let Some(byte_data_type) = byte_data_type else {
        return Classified::Skip;
    };
    match STRICT_BASE64.decode(data) {
        Ok(file_bytes) => Classified::Attachment(FileAttachment {
            file_bytes,
            byte_data_type,
        }),
        Err(_) => {
            tracing::warn!("Model Armor: skipping attachment with undecodable base64 content");
            Classified::Unscannable
        }
    }
}

fn parse_data_uri(raw: &str) -> (Option<&str>, Option<&str>) {
    let Some(rest) = raw.strip_prefix("data:") else {
        return (None, None);
    };
    let Some((header, data)) = rest.split_once(";base64,") else {
        return (None, None);
    };
    ((!header.is_empty()).then_some(header), Some(data))
}

fn mime_to_byte_data_type(mime: Option<&str>) -> Option<ByteDataType> {
    let mime = mime?;
    let media_type = mime
        .split_once(';')
        .map_or(mime, |(media_type, _)| media_type);
    let normalized = media_type.trim().to_lowercase();
    MIME_TO_BYTE_DATA_TYPE
        .iter()
        .find(|(candidate, _)| *candidate == normalized.as_str())
        .map(|(_, byte_data_type)| *byte_data_type)
}

fn is_remote_uri(raw: &str) -> bool {
    let lowered = raw.trim().to_lowercase();
    REMOTE_URI_SCHEMES
        .iter()
        .any(|scheme| lowered.starts_with(scheme))
}

mod tests;
