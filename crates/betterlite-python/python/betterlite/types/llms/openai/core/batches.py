from typing import Literal, TypeAlias, TypedDict

from typing_extensions import ReadOnly

from betterlite.types.llms.openai.core.files import FileExpiresAfter
from betterlite.types.llms.openai.core.not_given import NotGiven


class CreateBatchRequest(TypedDict, total=False):
    completion_window: Literal["24h"]
    endpoint: Literal[
        "/v1/chat/completions",
        "/v1/embeddings",
        "/v1/completions",
        "/v1/responses",
        "/v1/ocr",
        "/v1/images/generations",
        "/v1/images/edits",
        "/v1/videos/generations",
        "/v1/videos",
        "/v1/videos/edits",
        "/v1/videos/extensions",
    ]
    input_file_id: str
    metadata: dict[str, str] | None
    output_expires_after: FileExpiresAfter
    extra_headers: dict[str, str] | None
    extra_body: dict[str, str] | None
    timeout: float | None


class LiteLLMBatchCreateRequest(CreateBatchRequest, total=False):
    model: str
    disable_fallbacks: ReadOnly[bool]


class RetrieveBatchRequest(TypedDict, total=False):
    batch_id: str
    extra_headers: dict[str, str] | None
    extra_body: dict[str, str] | None
    timeout: float | None


class CancelBatchRequest(TypedDict, total=False):
    batch_id: str
    extra_headers: dict[str, str] | None
    extra_body: dict[str, str] | None
    timeout: float | None


class ListBatchRequest(TypedDict, total=False):
    after: str | NotGiven


class OpenAIErrorBody(TypedDict, total=False):
    error: dict[str, str]


BatchJobStatus: TypeAlias = Literal[
    "validating",
    "failed",
    "in_progress",
    "finalizing",
    "completed",
    "expired",
    "cancelling",
    "cancelled",
]
