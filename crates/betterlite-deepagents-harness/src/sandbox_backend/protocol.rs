use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Reply<T> {
    Done(T),
    Error(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileInfo {
    pub path: String,
    pub is_dir: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GrepMatch {
    pub path: String,
    pub line: usize,
    pub text: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Encoding {
    Utf8,
    Base64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileData {
    pub content: String,
    pub encoding: Encoding,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadWindow {
    pub total_lines: usize,
    pub start_line: usize,
    pub end_line: usize,
    // absent when the window reaches the last line
    pub next_offset: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReadResult {
    Error(String),
    Whole(FileData),
    NoLinesRequested,
    Window {
        file_data: FileData,
        window: ReadWindow,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Edited {
    pub path: String,
    pub occurrences: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Replacement<'text> {
    pub old: &'text str,
    pub new: &'text str,
    pub replace_all: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GrepQuery<'query> {
    pub pattern: &'query str,
    pub path: &'query str,
    // absent searches every file
    pub glob: Option<&'query str>,
    // absent keeps every match
    pub max_count: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Listing<T> {
    pub matches: Vec<T>,
    pub truncated: bool,
}

pub type LsResult = Reply<Vec<FileInfo>>;
pub type WriteResult = Reply<String>;
pub type EditResult = Reply<Edited>;
pub type DeleteResult = Reply<String>;
pub type GlobResult = Reply<Listing<FileInfo>>;
pub type GrepResult = Reply<Listing<GrepMatch>>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecuteResponse {
    pub output: String,
    pub exit_code: i32,
    pub truncated: bool,
}

#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum TransferError {
    #[error("file_not_found")]
    FileNotFound,
    #[error("permission_denied")]
    PermissionDenied,
    #[error("invalid_path")]
    InvalidPath,
    #[error("{0}")]
    Sandbox(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileUploadResponse {
    pub path: String,
    pub outcome: Result<(), TransferError>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileDownloadResponse {
    pub path: String,
    pub content: Result<Vec<u8>, TransferError>,
}
