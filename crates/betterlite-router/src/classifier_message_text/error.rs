use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ClassifierTextError {
    #[error("the text field of content part {part} is not a string")]
    PartTextNotString { part: usize },
    #[error("a content part image_url url is not a string")]
    ImageUrlNotString,
    #[error("a content part encrypted_content is not a string")]
    EncryptedContentNotString,
}
