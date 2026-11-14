from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from datetime import datetime
from enum import StrEnum
from typing import Literal

from pydantic import BaseModel, JsonValue
from typing_extensions import ReadOnly, TypedDict


# ast-grep-ignore: no-long-names
class SupportedVectorStoreIntegrations(StrEnum):
    BEDROCK = "bedrock"
    RAGFLOW = "ragflow"


class LiteLLM_VectorStoreConfig(TypedDict, total=False):
    vector_store_name: str
    litellm_params: dict[str, JsonValue] | None


class LiteLLM_ManagedVectorStore(TypedDict, total=False):
    vector_store_id: str
    custom_llm_provider: str

    vector_store_name: str | None
    vector_store_description: str | None
    vector_store_metadata: dict[str, JsonValue] | str | None
    created_at: datetime | None
    updated_at: datetime | None

    litellm_credential_name: str | None

    litellm_params: dict[str, JsonValue] | None

    team_id: str | None
    user_id: str | None

    is_config: ReadOnly[bool]


# ast-grep-ignore: no-long-names
class LiteLLM_ManagedVectorStoreListResponse(TypedDict, total=False):
    object: Literal["list"]
    data: list[LiteLLM_ManagedVectorStore]
    total_count: int | None
    current_page: int | None
    total_pages: int | None


class VectorStoreUpdateRequest(BaseModel):
    vector_store_id: str
    custom_llm_provider: str | None = None
    vector_store_name: str | None = None
    vector_store_description: str | None = None
    vector_store_metadata: dict[str, JsonValue] | None = None


class VectorStoreDeleteRequest(BaseModel):
    vector_store_id: str


class VectorStoreInfoRequest(BaseModel):
    vector_store_id: str


class VectorStoreResultContent(TypedDict, total=False):
    text: str | None
    type: str | None


class VectorStoreSearchResult(TypedDict, total=False):
    score: float | None
    content: list[VectorStoreResultContent] | None
    file_id: str | None
    filename: str | None
    attributes: dict[str, JsonValue] | None


class VectorStoreSearchResponse(TypedDict, total=False):
    object: Literal["vector_store.search_results.page"]
    search_query: str | None
    data: list[VectorStoreSearchResult] | None


VectorStoreSearchFailureMode = Literal["annotate", "error"]


class VectorStoreSearchFailure(TypedDict):
    vector_store_id: ReadOnly[str]
    custom_llm_provider: ReadOnly[str | None]
    error: ReadOnly[str]


# ast-grep-ignore: no-long-names
class VectorStoreSearchOptionalRequestParams(TypedDict, total=False):
    filters: dict[str, JsonValue] | None
    max_num_results: int | None
    ranking_options: dict[str, JsonValue] | None
    rewrite_query: bool | None


# ast-grep-ignore: no-long-names
class VectorStoreSearchRequest(VectorStoreSearchOptionalRequestParams, total=False):
    query: str | list[str]


class VertexSearchDataStoreExtraBody(TypedDict, total=False):
    query: str
    pageSize: int
    pageToken: str
    offset: int
    oneBoxPageSize: int
    pageCategories: Sequence[str]
    imageQuery: Mapping[str, JsonValue]
    filter: str
    canonicalFilter: str
    orderBy: str
    userInfo: Mapping[str, JsonValue]
    languageCode: str
    facetSpecs: Sequence[Mapping[str, JsonValue]]
    boostSpec: Mapping[str, JsonValue]
    params: Mapping[str, JsonValue]
    queryExpansionSpec: Mapping[str, JsonValue]
    spellCorrectionSpec: Mapping[str, JsonValue]
    userPseudoId: str
    contentSearchSpec: Mapping[str, JsonValue]
    rankingExpression: str
    rankingExpressionBackend: str
    safeSearch: bool
    userLabels: Mapping[str, str]
    # ast-grep-ignore: no-long-names
    naturalLanguageQueryUnderstandingSpec: Mapping[str, JsonValue]
    searchAsYouTypeSpec: Mapping[str, JsonValue]
    displaySpec: Mapping[str, JsonValue]
    crowdingSpecs: Sequence[Mapping[str, JsonValue]]
    relevanceThreshold: str
    relevanceScoreSpec: Mapping[str, JsonValue]
    customRankingParams: Mapping[str, JsonValue]


class VertexSearchEngineExtraBody(VertexSearchDataStoreExtraBody, total=False):
    dataStoreSpecs: Sequence[Mapping[str, JsonValue]]
    numResultsPerDataStore: int


class VectorStoreExpirationPolicy(TypedDict, total=False):
    anchor: Literal["last_active_at"]
    days: int


# ast-grep-ignore: no-long-names
class VectorStoreAutoChunkingStrategy(TypedDict, total=False):
    type: Literal["auto"]


# ast-grep-ignore: no-long-names
class VectorStoreStaticChunkingStrategyConfig(TypedDict, total=False):
    max_chunk_size_tokens: int
    chunk_overlap_tokens: int


# ast-grep-ignore: no-long-names
class VectorStoreStaticChunkingStrategy(TypedDict, total=False):
    type: Literal["static"]
    static: VectorStoreStaticChunkingStrategyConfig  # ast-grep-ignore: no-long-names


class VectorStoreChunkingStrategy(TypedDict, total=False):
    type: Literal["auto", "static"]
    static: VectorStoreStaticChunkingStrategyConfig | None  # ast-grep-ignore: no-long-names


class VectorStoreFileCounts(TypedDict, total=False):
    in_progress: int
    completed: int
    failed: int
    cancelled: int
    total: int


# ast-grep-ignore: no-long-names
class VectorStoreCreateOptionalRequestParams(TypedDict, total=False):
    name: str | None
    file_ids: list[str] | None
    expires_after: VectorStoreExpirationPolicy | None
    chunking_strategy: VectorStoreChunkingStrategy | None
    metadata: dict[str, str] | None


# ast-grep-ignore: no-long-names
class VectorStoreCreateRequest(VectorStoreCreateOptionalRequestParams, total=False):
    pass


class VectorStoreCreateResponse(TypedDict, total=False):
    id: str
    object: Literal["vector_store"]
    created_at: int
    name: str | None
    bytes: int
    file_counts: VectorStoreFileCounts
    status: Literal["expired", "in_progress", "completed"]
    expires_after: VectorStoreExpirationPolicy | None
    expires_at: int | None
    last_active_at: int | None
    metadata: dict[str, str] | None


class IndexCreateLiteLLMParams(BaseModel):
    vector_store_index: str
    vector_store_name: str


class IndexCreateRequest(BaseModel):
    index_name: str
    litellm_params: IndexCreateLiteLLMParams
    index_info: dict[str, JsonValue] | None = None


class BaseVectorStoreAuthCredentials(TypedDict, total=False):
    headers: dict[str, JsonValue]
    query_params: dict[str, JsonValue]


# ast-grep-ignore: no-long-names
class LiteLLM_ManagedVectorStoreIndex(BaseModel):
    id: str
    index_name: str
    litellm_params: IndexCreateLiteLLMParams
    index_info: dict[str, JsonValue] | None = None
    created_at: datetime | None = None
    created_by: str | None = None
    updated_at: datetime | None = None
    updated_by: str | None = None


class IndexListResponse(BaseModel):
    object: Literal["list"] = "list"
    data: tuple[LiteLLM_ManagedVectorStoreIndex, ...]  # ast-grep-ignore: no-long-names


class VectorStoreIndexType(StrEnum):
    READ = "read"
    WRITE = "write"


class VectorStoreIndexEndpoints(TypedDict):
    read: list[tuple[Literal["GET", "POST", "PUT", "DELETE", "PATCH"], str]]
    write: list[tuple[Literal["GET", "POST", "PUT", "DELETE", "PATCH"], str]]


VECTOR_STORE_OPENAI_PARAMS = Literal[
    "filters",
    "max_num_results",
    "ranking_options",
    "rewrite_query",
]


@dataclass
class VectorStoreToolParams:
    filters: dict[str, JsonValue] | None = None
    max_num_results: int | None = None
    ranking_options: dict[str, JsonValue] | None = None

    def to_dict(self) -> dict[str, JsonValue]:
        values: dict[str, JsonValue] = {
            "filters": self.filters,
            "max_num_results": self.max_num_results,
            "ranking_options": self.ranking_options,
        }
        return {key: value for key, value in values.items() if value is not None}
