use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum VideoContractError {
    #[error("api_base is required")]
    ApiBaseRequired,
    #[error("video create character is not supported for this provider")]
    CreateCharacterUnsupported,
    #[error("video get character is not supported for this provider")]
    GetCharacterUnsupported,
    #[error("video edit is not supported for this provider")]
    EditUnsupported,
    #[error("video extension is not supported for this provider")]
    ExtensionUnsupported,
}
