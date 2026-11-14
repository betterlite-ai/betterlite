import operator
from functools import partial
from typing import Final, Literal, Required, TypeAlias, TypedDict

from pydantic import Field, JsonValue, PrivateAttr

from betterlite.types.llms.openai.core.aliases import FileTypes
from betterlite.types.utils.base import DictJsonObject, ResponseObject


# ast-grep-ignore: no-long-names
OpenAICreateFileRequestOptionalParams: TypeAlias = Literal["purpose"]

OpenAIFilesPurpose: TypeAlias = Literal[
    "assistants",
    "assistants_output",
    "batch",
    "batch_output",
    "fine-tune",
    "fine-tune-results",
    "vision",
    "user_data",
    "evals",
    "messages",
]


class BatchGuardrailRecord(ResponseObject):
    line: int
    custom_id: str | None = None
    action: Literal["redacted", "dropped"]
    guardrail: str | None = None


class BatchGuardrailReport(ResponseObject):
    submitted_records: int
    modified_records: tuple[BatchGuardrailRecord, ...]


BATCH_GUARDRAIL_RESPONSE_FIELD: Final = "litellm_batch_guardrail"


class OpenAIFileObject(DictJsonObject):
    id: str
    bytes: int
    created_at: int
    filename: str
    object: Literal["file"]
    purpose: OpenAIFilesPurpose
    status: Literal["uploaded", "processed", "error", "pending"] | None = None
    expires_at: int | None = None
    status_details: str | None = None
    litellm_batch_guardrail: BatchGuardrailReport | None = Field(
        default=None, exclude_if=partial(operator.is_, None),
    )
    _hidden_params: dict[str, JsonValue] = PrivateAttr(default={"response_cost": 0.0})


class FileListPage(ResponseObject):
    object: Literal["list"] = "list"
    data: list[OpenAIFileObject] = Field(default_factory=list)
    first_id: str | None = None
    last_id: str | None = None
    has_more: bool = False


CREATE_FILE_REQUESTS_PURPOSE: TypeAlias = Literal["assistants", "batch", "fine-tune", "messages"]


class FileExpiresAfter(TypedDict):
    anchor: Required[Literal["created_at"]]
    seconds: Required[int]


class CreateFileRequest(TypedDict, total=False):
    file: Required[FileTypes]
    purpose: Required[CREATE_FILE_REQUESTS_PURPOSE]
    expires_after: FileExpiresAfter | None
    extra_headers: dict[str, str] | None
    extra_body: dict[str, str] | None
    timeout: float | None


class FileContentRequest(TypedDict, total=False):
    file_id: str
    extra_headers: dict[str, str] | None
    extra_body: dict[str, str] | None
    timeout: float | None
