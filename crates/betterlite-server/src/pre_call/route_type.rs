use thiserror::Error;

macro_rules! route_types {
    ($($variant:ident => $text:literal,)+) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum RouteType {
            $($variant,)+
        }

        impl RouteType {
            pub const ALL: &'static [Self] = &[$(Self::$variant,)+];

            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text,)+
                }
            }

            pub fn parse(text: &str) -> Result<Self, RouteTypeError> {
                Self::ALL
                    .iter()
                    .copied()
                    .find(|route| route.as_str() == text)
                    .ok_or_else(|| RouteTypeError(text.to_owned()))
            }
        }
    };
}

route_types! {
    Completion => "acompletion",
    Embedding => "aembedding",
    Responses => "aresponses",
    Realtime => "_arealtime",
    ResponsesWebsocket => "_aresponses_websocket",
    CreateRealtimeClientSecret => "acreate_realtime_client_secret",
    RealtimeCalls => "arealtime_calls",
    GetResponses => "aget_responses",
    DeleteResponses => "adelete_responses",
    CancelResponses => "acancel_responses",
    CompactResponses => "acompact_responses",
    CreateBatch => "acreate_batch",
    RetrieveBatch => "aretrieve_batch",
    ListBatches => "alist_batches",
    CancelBatch => "acancel_batch",
    FileContent => "afile_content",
    FileRetrieve => "afile_retrieve",
    FileDelete => "afile_delete",
    TextCompletion => "atext_completion",
    CreateFineTuningJob => "acreate_fine_tuning_job",
    CancelFineTuningJob => "acancel_fine_tuning_job",
    ListFineTuningJobs => "alist_fine_tuning_jobs",
    RetrieveFineTuningJob => "aretrieve_fine_tuning_job",
    ListInputItems => "alist_input_items",
    ImageEdit => "aimage_edit",
    GenerateContent => "agenerate_content",
    GenerateContentStream => "agenerate_content_stream",
    LlmPassthroughRoute => "allm_passthrough_route",
    VectorStoreSearch => "avector_store_search",
    VectorStoreCreate => "avector_store_create",
    VectorStoreRetrieve => "avector_store_retrieve",
    VectorStoreList => "avector_store_list",
    VectorStoreUpdate => "avector_store_update",
    VectorStoreDelete => "avector_store_delete",
    VectorStoreFileCreate => "avector_store_file_create",
    VectorStoreFileList => "avector_store_file_list",
    VectorStoreFileRetrieve => "avector_store_file_retrieve",
    VectorStoreFileContent => "avector_store_file_content",
    VectorStoreFileUpdate => "avector_store_file_update",
    VectorStoreFileDelete => "avector_store_file_delete",
    Ocr => "aocr",
    Search => "asearch",
    Decisions => "adecisions",
    Systemone => "asystemone",
    VideoGeneration => "avideo_generation",
    VideoList => "avideo_list",
    VideoStatus => "avideo_status",
    VideoContent => "avideo_content",
    VideoRemix => "avideo_remix",
    VideoCreateCharacter => "avideo_create_character",
    VideoGetCharacter => "avideo_get_character",
    VideoEdit => "avideo_edit",
    VideoExtension => "avideo_extension",
    CreateContainer => "acreate_container",
    ListContainers => "alist_containers",
    Ingest => "aingest",
    RetrieveContainer => "aretrieve_container",
    DeleteContainer => "adelete_container",
    UploadContainerFile => "aupload_container_file",
    ListContainerFiles => "alist_container_files",
    RetrieveContainerFile => "aretrieve_container_file",
    DeleteContainerFile => "adelete_container_file",
    RetrieveContainerFileContent => "aretrieve_container_file_content",
    CreateSkill => "acreate_skill",
    ListSkills => "alist_skills",
    GetSkill => "aget_skill",
    DeleteSkill => "adelete_skill",
    AnthropicMessages => "anthropic_messages",
    CreateInteraction => "acreate_interaction",
    GetInteraction => "aget_interaction",
    DeleteInteraction => "adelete_interaction",
    CancelInteraction => "acancel_interaction",
    CreateAgent => "acreate_agent",
    ListAgents => "alist_agents",
    GetAgent => "aget_agent",
    DeleteAgent => "adelete_agent",
    ListAgentVersions => "alist_agent_versions",
    SendMessage => "asend_message",
    CallMcpTool => "call_mcp_tool",
    CreateEval => "acreate_eval",
    ListEvals => "alist_evals",
    GetEval => "aget_eval",
    UpdateEval => "aupdate_eval",
    DeleteEval => "adelete_eval",
    CancelEval => "acancel_eval",
    CreateRun => "acreate_run",
    ListRuns => "alist_runs",
    GetRun => "aget_run",
    CancelRun => "acancel_run",
    DeleteRun => "adelete_run",
    ApplyGuardrail => "apply_guardrail",
    ImageGeneration => "aimage_generation",
    Moderation => "amoderation",
    Rerank => "arerank",
    Speech => "aspeech",
    Transcription => "atranscription",
    CreateRealtimeTranscription => "acreate_realtime_transcription_session",
}

#[derive(Debug, Error, PartialEq, Eq)]
#[error("unknown route type {0:?}")]
pub struct RouteTypeError(pub String);
