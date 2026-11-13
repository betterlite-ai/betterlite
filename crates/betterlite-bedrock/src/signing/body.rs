#[derive(Debug, Clone, Copy)]
pub enum Body<'source> {
    Text(&'source str),
    Bytes(&'source [u8]),
}
impl Body<'_> {
    #[must_use]
    pub const fn bytes(&self) -> &[u8] {
        match self {
            Self::Text(text) => text.as_bytes(),
            Self::Bytes(bytes) => bytes,
        }
    }
    // botocore measures a str in code points and a bytes payload in bytes
    #[must_use]
    pub fn content_length(&self) -> usize {
        match self {
            Self::Text(text) => text.chars().count(),
            Self::Bytes(bytes) => bytes.len(),
        }
    }
}
