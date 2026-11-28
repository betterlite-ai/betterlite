from typing import Final, Literal, NotRequired, Required, TypedDict

from litellm.types.rerank import RerankResponse
from litellm.types.utils import EmbeddingResponse, TokenCountResponse
from pydantic import JsonValue, TypeAdapter

import bedrock_other_count_call
import bedrock_other_embedding_jobs
import bedrock_other_rerank
import bedrock_other_transcription
from bedrock_image_dispatch.models import Input as ImageInput

Case = Literal[
    "embedding",
    "embedding_status",
    "rerank",
    "count",
    "transcription",
    "image_generation",
    "image_edit",
]


class Reply(TypedDict, total=False):
    status: int
    headers: dict[str, str]
    body: str
    failure: bool


class File(TypedDict):
    content: str
    filename: NotRequired[str]


class Probe(TypedDict, total=False):
    case: Required[Case]
    model: Required[str]
    input: list[str]
    prompt: str | None
    images: list[str]
    optional_params: dict[str, JsonValue]
    query: str | None
    documents: list[JsonValue]
    top_n: int | None
    return_documents: bool | None
    rank_fields: list[str] | None
    messages: list[dict[str, JsonValue]] | None
    request_model: str | None
    invocation_arn: str | None
    region: str | None
    environment: dict[str, str]
    file: File | None
    timeout: float | None
    timeout_ms: int | None
    reply_text: str | None
    asynchronous: bool
    api_base: str | None
    api_key: str | None
    extra_headers: dict[str, str] | None
    reply: Reply
    replies: list[Reply]


class Input(TypedDict):
    model_cost: NotRequired[dict[str, JsonValue]]
    probes: list[Probe]


class EmbeddingRequest(TypedDict):
    url: str
    headers: dict[str, str]
    body: JsonValue


class EmbeddingRecord(TypedDict):
    index: int
    embedding: JsonValue


class EmbeddingUsage(TypedDict):
    prompt_tokens: int
    total_tokens: int


class EmbeddingPayload(TypedDict):
    model: str | None
    data: list[EmbeddingRecord]
    usage: EmbeddingUsage


class EmbeddingOutput(TypedDict):
    ok: bool
    status: int | None
    requests: list[EmbeddingRequest]
    response: EmbeddingResponse | None


class StatusRequest(TypedDict):
    url: str
    headers: list[list[str]]
    body: str


class StatusOutput(TypedDict):
    requests: list[StatusRequest]
    resolved_regions: list[str]
    kind: Literal["returned", "failed"]
    value: JsonValue


class SignedHeaders(TypedDict):
    authorization: bool


class RerankRequest(TypedDict):
    posted_url: str
    posted_body: JsonValue
    signed_headers: SignedHeaders


class CountRequest(TypedDict):
    url: str
    body: str
    headers: dict[str, str]


class CountPayload(TypedDict):
    total_tokens: int
    request_model: str
    model_used: str
    tokenizer_type: str
    error: bool
    error_message: str | None
    status_code: int | None
    original_response: JsonValue


class CountCall(TypedDict):
    result: TokenCountResponse | None
    request: CountRequest | None


class CountOutput(TypedDict):
    calls: list[CountCall]


class TranscriptionResult(TypedDict):
    ok: bool
    seen: list[JsonValue]
    text: NotRequired[str]
    kind: NotRequired[str]


class TranscriptionOutput(TypedDict):
    calls: list[TranscriptionResult]


class ImageRequest(TypedDict):
    url: str | None
    body: JsonValue
    authorization: str | None
    posted: int


class ImageRow(TypedDict):
    request: ImageRequest
    ok: bool
    status: int | None
    images: list[JsonValue]
    cost: JsonValue


INPUT: Final[TypeAdapter[Input]] = TypeAdapter(Input)
JOBS_INPUT: Final[TypeAdapter[bedrock_other_embedding_jobs.Input]] = TypeAdapter(
    bedrock_other_embedding_jobs.Input
)
RERANK_PROBE: Final[TypeAdapter[bedrock_other_rerank.Probe]] = TypeAdapter(
    bedrock_other_rerank.Probe
)
COUNT_INPUT: Final[TypeAdapter[dict[str, bedrock_other_count_call.Calls]]] = TypeAdapter(
    dict[str, bedrock_other_count_call.Calls]
)
TRANSCRIPTION_INPUT: Final[TypeAdapter[bedrock_other_transcription.Calls]] = TypeAdapter(
    bedrock_other_transcription.Calls
)
IMAGE_INPUT: Final[TypeAdapter[ImageInput]] = TypeAdapter(ImageInput)
EMBEDDING_OUTPUT: Final[TypeAdapter[EmbeddingOutput]] = TypeAdapter(EmbeddingOutput)
EMBEDDING_PAYLOAD: Final[TypeAdapter[EmbeddingPayload]] = TypeAdapter(EmbeddingPayload)
STATUS_OUTPUT: Final[TypeAdapter[StatusOutput]] = TypeAdapter(StatusOutput)
RERANK_REQUEST: Final[TypeAdapter[RerankRequest]] = TypeAdapter(RerankRequest)
RERANK_OUTPUT: Final[TypeAdapter[RerankResponse]] = TypeAdapter(RerankResponse)
COUNT_OUTPUT: Final[TypeAdapter[CountOutput]] = TypeAdapter(CountOutput)
COUNT_PAYLOAD: Final[TypeAdapter[CountPayload]] = TypeAdapter(CountPayload)
TRANSCRIPTION_OUTPUT: Final[TypeAdapter[TranscriptionOutput]] = TypeAdapter(
    TranscriptionOutput
)
IMAGE_OUTPUT: Final[TypeAdapter[list[ImageRow]]] = TypeAdapter(list[ImageRow])
