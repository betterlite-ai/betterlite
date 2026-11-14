from collections.abc import Mapping
from typing import ClassVar, Literal

from pydantic import BaseModel, ConfigDict, JsonValue
from typing_extensions import ReadOnly, TypedDict

from betterlite.types.utils import ModelResponse


class RAGChunkingStrategy(TypedDict, total=False):
    chunk_size: int
    chunk_overlap: int
    separators: list[str] | None


class RAGIngestOCROptions(TypedDict, total=False):
    model: str


class RAGIngestEmbeddingOptions(TypedDict, total=False):
    model: str


class OpenAIVectorStoreOptions(TypedDict, total=False):
    custom_llm_provider: Literal["openai"]
    vector_store_id: str | None
    ttl_days: int | None

    litellm_credential_name: str | None
    api_key: str | None
    api_base: str | None


class BedrockVectorStoreOptions(TypedDict, total=False):
    custom_llm_provider: Literal["bedrock"]
    vector_store_id: str | None

    s3_bucket: str | None
    s3_prefix: str | None
    embedding_model: str | None
    data_source_id: str | None
    wait_for_ingestion: bool | None
    ingestion_timeout: int | None

    litellm_credential_name: str | None

    aws_access_key_id: str | None
    aws_secret_access_key: str | None
    aws_session_token: str | None
    aws_region_name: str | None
    aws_role_name: str | None
    aws_session_name: str | None
    aws_profile_name: str | None
    aws_web_identity_token: str | None
    aws_sts_endpoint: str | None
    aws_external_id: str | None


class VertexAIVectorStoreOptions(TypedDict, total=False):
    custom_llm_provider: Literal["vertex_ai"]
    vector_store_id: str

    vertex_project: str | None
    vertex_location: str | None
    vertex_credentials: str | None
    gcs_bucket: str | None

    wait_for_import: bool | None
    import_timeout: int | None


class S3VectorsVectorStoreOptions(TypedDict, total=False):
    custom_llm_provider: Literal["s3_vectors"]
    vector_bucket_name: str
    index_name: str | None

    dimension: int | None
    distance_metric: Literal["cosine", "euclidean"] | None
    non_filterable_metadata_keys: list[str] | None

    litellm_credential_name: str | None

    aws_access_key_id: str | None
    aws_secret_access_key: str | None
    aws_session_token: str | None
    aws_region_name: str | None
    aws_role_name: str | None
    aws_session_name: str | None
    aws_profile_name: str | None
    aws_web_identity_token: str | None
    aws_sts_endpoint: str | None
    aws_external_id: str | None


RAGIngestVectorStoreOptions = (
    OpenAIVectorStoreOptions
    | BedrockVectorStoreOptions
    | VertexAIVectorStoreOptions
    | S3VectorsVectorStoreOptions
)


class RAGIngestOptions(TypedDict, total=False):
    name: str | None
    ocr: RAGIngestOCROptions | None
    chunking_strategy: RAGChunkingStrategy | None
    embedding: RAGIngestEmbeddingOptions | None
    vector_store: RAGIngestVectorStoreOptions


class RAGIngestResponse(TypedDict, total=False):
    id: str
    status: Literal["completed", "in_progress", "failed"]
    vector_store_id: str
    file_id: str | None
    error: str | None


class RAGIngestRequest(BaseModel):
    file_url: str | None = None
    file_id: str | None = None
    ingest_options: dict[str, JsonValue]

    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow")


class RAGRetrievalConfig(TypedDict, total=False):
    vector_store_id: ReadOnly[str]
    custom_llm_provider: ReadOnly[str]
    top_k: ReadOnly[int]
    filters: ReadOnly[Mapping[str, JsonValue] | None]
    retrieval_filter: ReadOnly[Mapping[str, JsonValue] | None]


class RAGRerankConfig(TypedDict, total=False):
    enabled: bool
    model: str
    top_n: int
    return_documents: bool | None


class RAGQueryRequest(BaseModel):
    model: str
    messages: list[JsonValue]
    retrieval_config: RAGRetrievalConfig
    rerank: RAGRerankConfig | None = None
    stream: bool | None = False

    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow")


class RAGQueryResponse(ModelResponse):
    pass
