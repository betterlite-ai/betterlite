use betterlite_openai::files::records::FileStatus;

// The states types/llms/gemini.py declares; any other provider string maps like STATE_UNSPECIFIED.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileState {
    Unspecified,
    Processing,
    Active,
    Failed,
}

impl FileState {
    #[must_use]
    pub(crate) fn from_wire(state: Option<&str>) -> Self {
        match state {
            Some("ACTIVE") => Self::Active,
            Some("FAILED") => Self::Failed,
            Some("PROCESSING") => Self::Processing,
            _ => Self::Unspecified,
        }
    }

    #[must_use]
    pub(crate) const fn status(self) -> FileStatus {
        match self {
            Self::Active => FileStatus::Processed,
            Self::Failed => FileStatus::Error,
            Self::Unspecified | Self::Processing => FileStatus::Uploaded,
        }
    }
}
