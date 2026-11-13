use betterlite_core::routes::CallType;

pub const COMPLETION_CALL_TYPES: [CallType; 2] =
    [CallType::TextCompletion, CallType::AtextCompletion];

pub const EMBEDDINGS_CALL_TYPES: [CallType; 2] = [CallType::Embedding, CallType::Aembedding];

pub const IMAGE_CALL_TYPES: [CallType; 2] = [CallType::ImageGeneration, CallType::AimageGeneration];

pub const SPEECH_CALL_TYPES: [CallType; 2] = [CallType::Speech, CallType::Aspeech];

pub const TRANSCRIPTION_CALL_TYPES: [CallType; 2] =
    [CallType::Transcription, CallType::Atranscription];

pub const VIDEO_CALL_TYPES: [CallType; 10] = [
    CallType::VideoGeneration,
    CallType::AvideoGeneration,
    CallType::CreateVideo,
    CallType::AcreateVideo,
    CallType::VideoRemix,
    CallType::AvideoRemix,
    CallType::VideoEdit,
    CallType::AvideoEdit,
    CallType::VideoExtension,
    CallType::AvideoExtension,
];

mod tests;
