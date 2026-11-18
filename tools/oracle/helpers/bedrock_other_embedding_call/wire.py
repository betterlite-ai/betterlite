import inspect
from collections.abc import Awaitable
from typing import Literal, NotRequired, TypedDict, cast

import httpx
from litellm.types.utils import EmbeddingResponse
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter


class Reply(TypedDict):
    status: int
    body: str
    headers: NotRequired[dict[str, str]]
    failure: NotRequired[Literal["timeout", "failed"]]


class Case(TypedDict):
    function: str
    model: str
    input: list[str]
    now: int
    optional_params: NotRequired[dict[str, JsonValue]]
    api_base: NotRequired[str | None]
    extra_headers: NotRequired[dict[str, str] | None]
    api_key: NotRequired[str | None]
    timeout: NotRequired[float | None]
    aembedding: NotRequired[bool]
    drop_params: NotRequired[bool]
    environment: NotRequired[dict[str, str]]
    replies: NotRequired[list[Reply]]
    supplied: NotRequired[Literal["none", "sync", "async"]]


class Record(TypedDict):
    object: str
    index: int
    embedding: list[JsonValue] | str


class Usage(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int
    prompt_tokens_details: dict[str, JsonValue] | None


class Payload(TypedDict):
    object: str
    data: list[Record]
    model: str | None
    usage: Usage


class TimeoutView(BaseModel):
    model_config = ConfigDict(from_attributes=True, arbitrary_types_allowed=True)
    connect: float
    read: float
    write: float
    pool: float


CASE: TypeAdapter[Case] = TypeAdapter(Case)
PAYLOAD: TypeAdapter[Payload] = TypeAdapter(Payload)
BODY: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
TIMEOUT: TypeAdapter[httpx.Timeout | float | None] = TypeAdapter(
    httpx.Timeout | float | None,
    config=ConfigDict(arbitrary_types_allowed=True),
)


SKIPPED = frozenset(
    {"host", "content-length", "user-agent", "accept-encoding", "connection", "transfer-encoding"}
)


def request_value(request: httpx.Request) -> JsonValue:
    headers: dict[str, JsonValue] = {}
    for name, value in request.headers.items():
        lowered = name.lower()
        if lowered in SKIPPED or (lowered == "accept" and value == "*/*"):
            continue
        headers[lowered] = value.split(", Signature=")[0]
    return {
        "url": str(request.url),
        "headers": headers,
        "body": BODY.validate_json(request.content),
    }


def timeout_fields(timeout: JsonValue) -> list[JsonValue] | None:
    checked = TIMEOUT.validate_python(timeout)
    if checked is None:
        return None
    view: TimeoutView
    match checked:
        case httpx.Timeout() as value:
            view = TimeoutView.model_validate(value)
        case float() | int() as seconds:
            view = TimeoutView.model_validate(httpx.Timeout(seconds))
    return [view.connect, view.read, view.write, view.pool]


# aembedding hands back a coroutine on the batch and cohere paths
async def settle(
    pending: EmbeddingResponse | Awaitable[EmbeddingResponse],
) -> EmbeddingResponse:
    if inspect.isawaitable(pending):
        return await pending
    return pending


def response_value(response: EmbeddingResponse) -> JsonValue:
    payload = PAYLOAD.validate_json(response.model_dump_json())
    hidden = cast(dict[str, JsonValue], response._hidden_params)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2311
    usage = payload["usage"]
    details = usage["prompt_tokens_details"]
    counts: dict[str, JsonValue] | None = None
    if details is not None:
        counts = {name: value for name, value in details.items() if value is not None}
    return {
        "object": payload["object"],
        "data": [
            {"object": r["object"], "index": r["index"], "embedding": r["embedding"]}
            for r in payload["data"]
        ],
        "model": payload["model"],
        "usage": {
            "prompt_tokens": usage["prompt_tokens"],
            "completion_tokens": usage["completion_tokens"],
            "total_tokens": usage["total_tokens"],
            "prompt_tokens_details": counts,
        },
        "invocation_arn": hidden.get("_invocation_arn"),
    }
