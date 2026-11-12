use betterlite_openai::api::file_type_records::ByteStream;

pub trait BaseFileUploadStream: Send + Sync {
    fn iter_bytes(&self) -> ByteStream;
}
