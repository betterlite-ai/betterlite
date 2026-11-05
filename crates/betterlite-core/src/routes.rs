use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CallType {
    Embedding,
    Aembedding,
    Completion,
    Acompletion,
    AtextCompletion,
    TextCompletion,
    ImageGeneration,
    AimageGeneration,
    ImageEdit,
    AimageEdit,
    Moderation,
    Amoderation,
    Atranscription,
    Transcription,
    Aspeech,
    Speech,
    Rerank,
    Arerank,
    Search,
    Asearch,
    Decisions,
    Adecisions,
    Systemone,
    Asystemone,
    #[serde(rename = "_arealtime")]
    PrivateArealtime,
    #[serde(rename = "_aresponses_websocket")]
    PrivateAresponsesWebsocket,
    CreateBatch,
    AcreateBatch,
    AretrieveBatch,
    RetrieveBatch,
    AcancelBatch,
    CancelBatch,
    PassThroughEndpoint,
    AnthropicMessages,
    AanthropicMessages,
    GetAssistants,
    AgetAssistants,
    CreateAssistants,
    AcreateAssistants,
    DeleteAssistant,
    AdeleteAssistant,
    AcreateThread,
    CreateThread,
    AgetThread,
    GetThread,
    AAddMessage,
    AddMessage,
    AgetMessages,
    GetMessages,
    ArunThread,
    RunThread,
    ArunThreadStream,
    RunThreadStream,
    AfileRetrieve,
    FileRetrieve,
    AfileDelete,
    FileDelete,
    AfileList,
    FileList,
    AcreateFile,
    CreateFile,
    AfileContent,
    FileContent,
    CreateFineTuningJob,
    AcreateFineTuningJob,
    CreateVideo,
    AcreateVideo,
    VideoGeneration,
    AvideoGeneration,
    AvideoRetrieve,
    VideoRetrieve,
    AvideoContent,
    VideoContent,
    VideoRemix,
    AvideoRemix,
    VideoList,
    AvideoList,
    VideoRetrieveJob,
    AvideoRetrieveJob,
    VideoDelete,
    AvideoDelete,
    VideoCreateCharacter,
    AvideoCreateCharacter,
    VideoGetCharacter,
    AvideoGetCharacter,
    VideoEdit,
    AvideoEdit,
    VideoExtension,
    AvideoExtension,
    VectorStoreFileCreate,
    AvectorStoreFileCreate,
    VectorStoreFileList,
    AvectorStoreFileList,
    VectorStoreFileRetrieve,
    AvectorStoreFileRetrieve,
    VectorStoreFileContent,
    AvectorStoreFileContent,
    VectorStoreFileUpdate,
    AvectorStoreFileUpdate,
    VectorStoreFileDelete,
    AvectorStoreFileDelete,
    VectorStoreCreate,
    AvectorStoreCreate,
    VectorStoreSearch,
    AvectorStoreSearch,
    Ingest,
    Aingest,
    Query,
    Aquery,
    CreateInteraction,
    AcreateInteraction,
    CreateContainer,
    AcreateContainer,
    ListContainers,
    AlistContainers,
    RetrieveContainer,
    AretrieveContainer,
    DeleteContainer,
    AdeleteContainer,
    ListContainerFiles,
    AlistContainerFiles,
    UploadContainerFile,
    AuploadContainerFile,
    CreateSandbox,
    AcreateSandbox,
    DeleteSandbox,
    AdeleteSandbox,
    RunCode,
    ArunCode,
    CodeInterpreterTool,
    AcodeInterpreterTool,
    AcancelFineTuningJob,
    CancelFineTuningJob,
    AlistFineTuningJobs,
    ListFineTuningJobs,
    AretrieveFineTuningJob,
    RetrieveFineTuningJob,
    Responses,
    Aresponses,
    AlistInputItems,
    LlmPassthroughRoute,
    AllmPassthroughRoute,
    GenerateContent,
    AgenerateContent,
    GenerateContentStream,
    AgenerateContentStream,
    Ocr,
    Aocr,
    CallMcpTool,
    ListMcpTools,
    AsendMessage,
    SendMessage,
    AcreateSkill,
}

#[derive(Debug, Error)]
pub enum RouteMetadataError {
    #[error("route exceeds the 4096 byte limit")]
    RouteTooLong,
    #[error("route has more than 256 segments")]
    TooManySegments,
    #[error("pinned route metadata fixture exceeds the 128 KiB limit")]
    CatalogTooLong,
    #[error("pinned route metadata fixture is invalid: {0}")]
    Fixture(#[from] serde_json::Error),
    #[error("pinned route metadata fixture contains an undeclared call type")]
    UndeclaredCallType,
    #[error("pinned route metadata fixture contains a route without call types")]
    EmptyRouteTypes,
}

pub mod catalog;
pub mod pattern;

pub fn get_call_types_for_route(route: &str) -> Result<Option<Vec<CallType>>, RouteMetadataError> {
    let catalog = catalog::RouteCatalog::parse(catalog::CATALOG)?;
    if route.len() > 4096 {
        return Err(RouteMetadataError::RouteTooLong);
    }
    if let Some(exact) = catalog.routes.iter().find(|entry| entry.route == route) {
        return Ok(Some(exact.call_types.clone()));
    }
    for entry in &catalog.routes {
        if pattern::pattern_matches(route, &entry.route)? {
            return Ok(Some(entry.call_types.clone()));
        }
    }
    Ok(None)
}

pub fn get_routes_for_call_type(call_type: CallType) -> Result<Vec<String>, RouteMetadataError> {
    let catalog = catalog::RouteCatalog::parse(catalog::CATALOG)?;
    Ok(catalog
        .routes
        .into_iter()
        .filter(|entry| entry.call_types.contains(&call_type))
        .map(|entry| entry.route)
        .collect())
}

mod tests;
