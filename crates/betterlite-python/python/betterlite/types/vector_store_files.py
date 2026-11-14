from enum import StrEnum
from typing import Literal

from pydantic import JsonValue
from typing_extensions import TypedDict


class VectorStoreFileStatus(StrEnum):
    IN_PROGRESS = "in_progress"
    COMPLETED = "completed"
    FAILED = "failed"
    CANCELLED = "cancelled"


# ast-grep-ignore: no-long-names
class VectorStoreFileChunkingStrategyType(StrEnum):
    AUTO = "auto"
    STATIC = "static"


# ast-grep-ignore: no-long-names
class VectorStoreFileStaticChunkingConfig(TypedDict, total=False):
    max_chunk_size_tokens: int
    chunk_overlap_tokens: int


# ast-grep-ignore: no-long-names
class VectorStoreFileChunkingStrategy(TypedDict, total=False):
    type: Literal["auto", "static"]
    static: VectorStoreFileStaticChunkingConfig | None  # ast-grep-ignore: no-long-names


class VectorStoreFileObject(TypedDict, total=False):
    id: str
    object: Literal["vector_store.file"]
    created_at: int
    usage_bytes: int | None
    vector_store_id: str
    status: VectorStoreFileStatus
    last_error: dict[str, JsonValue] | None
    chunking_strategy: VectorStoreFileChunkingStrategy | None  # ast-grep-ignore: no-long-names
    attributes: dict[str, str | int | float | bool] | None


class VectorStoreFileCreateRequest(TypedDict, total=False):
    file_id: str
    attributes: dict[str, str | int | float | bool] | None
    chunking_strategy: VectorStoreFileChunkingStrategy | None  # ast-grep-ignore: no-long-names


class VectorStoreFileUpdateRequest(TypedDict, total=False):
    attributes: dict[str, str | int | float | bool]


class VectorStoreFileListQueryParams(TypedDict, total=False):
    after: str | None
    before: str | None
    filter: Literal["in_progress", "completed", "failed", "cancelled"] | None
    limit: int | None
    order: Literal["asc", "desc"] | None


class VectorStoreFileListResponse(TypedDict, total=False):
    object: Literal["list"]
    data: list[VectorStoreFileObject]
    first_id: str | None
    last_id: str | None
    has_more: bool


class VectorStoreFileDeleteResponse(TypedDict, total=False):
    id: str
    object: Literal["vector_store.file.deleted"]
    deleted: bool


class VectorStoreFileContentTextPart(TypedDict, total=False):
    type: Literal["text"]
    text: str


class VectorStoreFileContentResponse(TypedDict, total=False):
    file_id: str
    filename: str | None
    attributes: dict[str, str | int | float | bool] | None
    content: list[VectorStoreFileContentTextPart]


class VectorStoreFileAuthCredentials(TypedDict, total=False):
    headers: dict[str, JsonValue]
    query_params: dict[str, JsonValue]
