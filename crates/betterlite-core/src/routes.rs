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

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct RouteEntry {
    pub route: String,
    pub call_types: Vec<CallType>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct RouteCatalog {
    pub source: String,
    pub call_types: Vec<String>,
    pub routes: Vec<RouteEntry>,
}

#[derive(Debug, Error)]
pub enum RouteMetadataError {
    #[error("route exceeds the 4096 byte limit")]
    RouteTooLong,
    #[error("route has more than 256 segments")]
    TooManySegments,
    #[error("pinned route metadata fixture is invalid: {0}")]
    Fixture(#[from] serde_json::Error),
    #[error("pinned route metadata fixture contains an undeclared call type")]
    UndeclaredCallType,
    #[error("pinned route metadata fixture contains a route without call types")]
    EmptyRouteTypes,
}

const ROUTE_CATALOG: &str = include_str!("../fixtures/route_metadata.json");

pub fn route_catalog() -> Result<RouteCatalog, RouteMetadataError> {
    let catalog: RouteCatalog = serde_json::from_str(ROUTE_CATALOG)?;
    for entry in &catalog.routes {
        if entry.call_types.is_empty() {
            return Err(RouteMetadataError::EmptyRouteTypes);
        }
        for call_type in &entry.call_types {
            let value = serde_json::to_value(call_type)?;
            if !value
                .as_str()
                .is_some_and(|name| catalog.call_types.iter().any(|known| known == name))
            {
                return Err(RouteMetadataError::UndeclaredCallType);
            }
        }
    }
    Ok(catalog)
}

pub fn parse_route(route: &str) -> Result<Vec<&str>, RouteMetadataError> {
    if route.len() > 4096 {
        return Err(RouteMetadataError::RouteTooLong);
    }
    let trimmed = route.trim_matches('/');
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let segments = trimmed.split('/').collect::<Vec<_>>();
    if segments.len() > 256 {
        return Err(RouteMetadataError::TooManySegments);
    }
    Ok(segments)
}

fn segment_matches(route: &str, pattern: &str) -> bool {
    if !pattern.starts_with('{') {
        return route == pattern;
    }
    let Some(end) = pattern.find('}') else {
        return route == pattern;
    };
    let suffix = &pattern[end + 1..];
    if suffix.is_empty() {
        return true;
    }
    route.ends_with(suffix) && route.len() > suffix.len()
}

fn pattern_matches(route: &str, pattern: &str) -> Result<bool, RouteMetadataError> {
    let route_parts = parse_route(route)?;
    let pattern_parts = parse_route(pattern)?;
    if pattern_parts.is_empty() || route_parts.len() < pattern_parts.len() {
        return Ok(false);
    }
    let Some(tail) = pattern_parts.last() else {
        return Ok(false);
    };
    let spans = tail.starts_with('{') && tail.contains('}') && !tail.ends_with('}');
    if route_parts.len() > pattern_parts.len() && !spans {
        return Ok(false);
    }
    let head_count = pattern_parts.len() - 1;
    for (route_part, pattern_part) in route_parts
        .iter()
        .take(head_count)
        .zip(pattern_parts.iter())
    {
        if !segment_matches(route_part, pattern_part) {
            return Ok(false);
        }
    }
    let tail_value = route_parts
        .iter()
        .skip(head_count)
        .copied()
        .collect::<Vec<_>>()
        .join("/");
    Ok(segment_matches(&tail_value, tail))
}

pub fn get_call_types_for_route(route: &str) -> Result<Option<Vec<CallType>>, RouteMetadataError> {
    let catalog = route_catalog()?;
    if route.len() > 4096 {
        return Err(RouteMetadataError::RouteTooLong);
    }
    if let Some(exact) = catalog.routes.iter().find(|entry| entry.route == route) {
        return Ok(Some(exact.call_types.clone()));
    }
    for entry in &catalog.routes {
        if pattern_matches(route, &entry.route)? {
            return Ok(Some(entry.call_types.clone()));
        }
    }
    Ok(None)
}

pub fn get_routes_for_call_type(call_type: CallType) -> Result<Vec<String>, RouteMetadataError> {
    let catalog = route_catalog()?;
    Ok(catalog
        .routes
        .into_iter()
        .filter(|entry| entry.call_types.contains(&call_type))
        .map(|entry| entry.route)
        .collect())
}

mod tests;
