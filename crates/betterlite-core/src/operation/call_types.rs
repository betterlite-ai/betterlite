use crate::routes::CallType;

use super::Entry;

// a call type maps to a dispatch entry; TextStream has no type of its own, it streams a text completion
#[rustfmt::skip]
const EXECUTABLE: [(Entry, &[CallType]); 18] = [
    (Entry::Chat, &[CallType::Completion, CallType::Acompletion]),
    (Entry::Embedding, &[CallType::Embedding, CallType::Aembedding]),
    (Entry::TextCompletion, &[CallType::TextCompletion, CallType::AtextCompletion]),
    (Entry::Moderation, &[CallType::Moderation, CallType::Amoderation]),
    (Entry::Responses, &[CallType::Responses, CallType::Aresponses]),
    (Entry::ImageGeneration, &[CallType::ImageGeneration, CallType::AimageGeneration]),
    (Entry::ImageEdit, &[CallType::ImageEdit, CallType::AimageEdit]),
    (Entry::VideoGeneration, &[CallType::VideoGeneration, CallType::AvideoGeneration]),
    (Entry::VideoList, &[CallType::VideoList, CallType::AvideoList]),
    (Entry::VideoStatus, &[CallType::VideoRetrieve, CallType::AvideoRetrieve]),
    (Entry::VideoContent, &[CallType::VideoContent, CallType::AvideoContent]),
    (Entry::VideoRemix, &[CallType::VideoRemix, CallType::AvideoRemix]),
    (Entry::VideoCreateCharacter, &[CallType::VideoCreateCharacter, CallType::AvideoCreateCharacter]),
    (Entry::VideoGetCharacter, &[CallType::VideoGetCharacter, CallType::AvideoGetCharacter]),
    (Entry::VideoEdit, &[CallType::VideoEdit, CallType::AvideoEdit]),
    (Entry::VideoExtension, &[CallType::VideoExtension, CallType::AvideoExtension]),
    (Entry::LlmPassthrough, &[CallType::LlmPassthroughRoute, CallType::AllmPassthroughRoute]),
    (Entry::ContainerCall, &[CallType::CreateContainer, CallType::AcreateContainer, CallType::ListContainers, CallType::AlistContainers, CallType::RetrieveContainer, CallType::AretrieveContainer, CallType::DeleteContainer, CallType::AdeleteContainer, CallType::ListContainerFiles, CallType::AlistContainerFiles, CallType::UploadContainerFile, CallType::AuploadContainerFile]),
];

#[must_use]
pub fn entry_for(call_type: CallType) -> Option<Entry> {
    EXECUTABLE
        .iter()
        .find_map(|(entry, types)| types.contains(&call_type).then_some(*entry))
}
