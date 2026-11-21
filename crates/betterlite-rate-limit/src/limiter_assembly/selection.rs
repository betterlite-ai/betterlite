use betterlite_core::routes::CallType;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LimiterChoice {
    Main,
    Batch,
}

// LiteLLM's get_rate_limiter_for_call_type: only acreate_batch hands the call to the batch limiter
#[must_use]
#[rustfmt::skip]
pub const fn select_call_type_limiter(call_type: CallType) -> LimiterChoice {
    match call_type {
        CallType::AcreateBatch => LimiterChoice::Batch,
        CallType::Embedding | CallType::Aembedding | CallType::Completion | CallType::Acompletion | CallType::AtextCompletion | CallType::TextCompletion
        | CallType::ImageGeneration | CallType::AimageGeneration | CallType::ImageEdit | CallType::AimageEdit | CallType::Moderation | CallType::Amoderation
        | CallType::Atranscription | CallType::Transcription | CallType::Aspeech | CallType::Speech | CallType::Rerank | CallType::Arerank | CallType::Search
        | CallType::Asearch | CallType::Decisions | CallType::Adecisions | CallType::Systemone | CallType::Asystemone | CallType::PrivateArealtime
        | CallType::PrivateAresponsesWebsocket | CallType::CreateBatch | CallType::AretrieveBatch | CallType::RetrieveBatch | CallType::AcancelBatch
        | CallType::CancelBatch | CallType::PassThroughEndpoint | CallType::AnthropicMessages | CallType::AanthropicMessages | CallType::GetAssistants
        | CallType::AgetAssistants | CallType::CreateAssistants | CallType::AcreateAssistants | CallType::DeleteAssistant | CallType::AdeleteAssistant
        | CallType::AcreateThread | CallType::CreateThread | CallType::AgetThread | CallType::GetThread | CallType::AAddMessage | CallType::AddMessage
        | CallType::AgetMessages | CallType::GetMessages | CallType::ArunThread | CallType::RunThread | CallType::ArunThreadStream | CallType::RunThreadStream
        | CallType::AfileRetrieve | CallType::FileRetrieve | CallType::AfileDelete | CallType::FileDelete | CallType::AfileList | CallType::FileList
        | CallType::AcreateFile | CallType::CreateFile | CallType::AfileContent | CallType::FileContent | CallType::CreateFineTuningJob
        | CallType::AcreateFineTuningJob | CallType::CreateVideo | CallType::AcreateVideo | CallType::VideoGeneration | CallType::AvideoGeneration
        | CallType::AvideoRetrieve | CallType::VideoRetrieve | CallType::AvideoContent | CallType::VideoContent | CallType::VideoRemix | CallType::AvideoRemix
        | CallType::VideoList | CallType::AvideoList | CallType::VideoRetrieveJob | CallType::AvideoRetrieveJob | CallType::VideoDelete | CallType::AvideoDelete
        | CallType::VideoCreateCharacter | CallType::AvideoCreateCharacter | CallType::VideoGetCharacter | CallType::AvideoGetCharacter | CallType::VideoEdit
        | CallType::AvideoEdit | CallType::VideoExtension | CallType::AvideoExtension | CallType::VectorStoreFileCreate | CallType::AvectorStoreFileCreate
        | CallType::VectorStoreFileList | CallType::AvectorStoreFileList | CallType::VectorStoreFileRetrieve | CallType::AvectorStoreFileRetrieve
        | CallType::VectorStoreFileContent | CallType::AvectorStoreFileContent | CallType::VectorStoreFileUpdate | CallType::AvectorStoreFileUpdate
        | CallType::VectorStoreFileDelete | CallType::AvectorStoreFileDelete | CallType::VectorStoreCreate | CallType::AvectorStoreCreate
        | CallType::VectorStoreSearch | CallType::AvectorStoreSearch | CallType::Ingest | CallType::Aingest | CallType::Query | CallType::Aquery
        | CallType::CreateInteraction | CallType::AcreateInteraction | CallType::CreateContainer | CallType::AcreateContainer | CallType::ListContainers
        | CallType::AlistContainers | CallType::RetrieveContainer | CallType::AretrieveContainer | CallType::DeleteContainer | CallType::AdeleteContainer
        | CallType::ListContainerFiles | CallType::AlistContainerFiles | CallType::UploadContainerFile | CallType::AuploadContainerFile
        | CallType::CreateSandbox | CallType::AcreateSandbox | CallType::DeleteSandbox | CallType::AdeleteSandbox | CallType::RunCode | CallType::ArunCode
        | CallType::CodeInterpreterTool | CallType::AcodeInterpreterTool | CallType::AcancelFineTuningJob | CallType::CancelFineTuningJob
        | CallType::AlistFineTuningJobs | CallType::ListFineTuningJobs | CallType::AretrieveFineTuningJob | CallType::RetrieveFineTuningJob
        | CallType::Responses | CallType::Aresponses | CallType::AlistInputItems | CallType::LlmPassthroughRoute | CallType::AllmPassthroughRoute
        | CallType::GenerateContent | CallType::AgenerateContent | CallType::GenerateContentStream | CallType::AgenerateContentStream | CallType::Ocr
        | CallType::Aocr | CallType::CallMcpTool | CallType::ListMcpTools | CallType::AsendMessage | CallType::SendMessage | CallType::AcreateSkill => LimiterChoice::Main,
    }
}
