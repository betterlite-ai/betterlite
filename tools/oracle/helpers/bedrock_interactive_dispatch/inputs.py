from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

from litellm.types.llms.openai import (  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L59
    ResponseInputParam,
)
from pydantic import JsonValue, TypeAdapter

JsonObject = dict[str, JsonValue]
ResponsesSource: TypeAlias = str | ResponseInputParam

KEYS: Final[tuple[str, ...]] = (
    "request", "endpoint_url", "streaming", "status", "reply", "response", "logged", "error"
)


class UrlView(TypedDict):
    url: str
    endpoint_url: str
    streaming: bool


class Condensed(TypedDict):
    content: JsonValue
    finish_reason: JsonValue


class CollectView(TypedDict):
    view: Condensed | None


class MappedParams(TypedDict):
    params: JsonObject


class MappedResult(TypedDict):
    results: list[MappedParams]


class VectorCall(TypedDict):
    url: str
    body: JsonObject
    query: str


class VectorCalls(TypedDict):
    calls: list[VectorCall]


class VectorValue(TypedDict):
    value: JsonObject


class VectorResult(TypedDict):
    responses: list[VectorValue]


class RealtimeSeen(TypedDict):
    outcome: str
    error_kind: str | None
    status_code: int | None
    committed: bool


class RealtimeResult(TypedDict):
    runs: list[RealtimeSeen]


class ResponsesRun(TypedDict):
    model: str
    input: JsonValue
    params: JsonObject
    api_key: NotRequired[str | None]
    status: int
    reply: str


class PassthroughRun(TypedDict):
    endpoint: str
    method: NotRequired[str]
    data: JsonObject
    region: NotRequired[str | None]
    status: int
    reply: str
    chunks: NotRequired[list[str]]


class SearchRun(TypedDict):
    query: str
    optional: JsonObject
    tool_name: NotRequired[str | None]
    api_key: NotRequired[str | None]
    status: int
    reply: str


class VectorRun(TypedDict):
    query: str
    optional: JsonObject
    extra_body: NotRequired[JsonObject | None]
    status: int
    reply: str


class RealtimeRun(TypedDict):
    pending: str
    region: NotRequired[str | None]


class ResponsesCase(TypedDict):
    case: Literal["responses"]
    runs: list[ResponsesRun]


class PassthroughCase(TypedDict):
    case: Literal["passthrough"]
    runs: list[PassthroughRun]


class SearchCase(TypedDict):
    case: Literal["search"]
    runs: list[SearchRun]


class VectorCase(TypedDict):
    case: Literal["vector_retrieve"]
    runs: list[VectorRun]


class RealtimeCase(TypedDict):
    case: Literal["realtime_bridge"]
    runs: list[RealtimeRun]


Case = ResponsesCase | PassthroughCase | SearchCase | VectorCase | RealtimeCase

URL_VIEW: Final[TypeAdapter[UrlView]] = TypeAdapter(UrlView)
COLLECT_VIEW: Final[TypeAdapter[CollectView]] = TypeAdapter(CollectView)
MAPPED_RESULT: Final[TypeAdapter[MappedResult]] = TypeAdapter(MappedResult)
VECTOR_CALLS: Final[TypeAdapter[VectorCalls]] = TypeAdapter(VectorCalls)
VECTOR_RESULT: Final[TypeAdapter[VectorResult]] = TypeAdapter(VectorResult)
REALTIME_RESULT: Final[TypeAdapter[RealtimeResult]] = TypeAdapter(RealtimeResult)
RESPONSES_SOURCE: Final[TypeAdapter[ResponsesSource]] = TypeAdapter(ResponsesSource)


def authorization_view(headers: dict[str, str]) -> JsonValue:
    value = headers.get("Authorization")
    if value is None:
        return None
    if value.startswith("Bearer "):
        return {
            "kind": "bearer", "access_key": None, "region": None,
            "service": None, "token": value[len("Bearer ") :],
        }
    scope = value.partition("Credential=")[2].partition(",")[0]
    access_key, _date, region, service, _scope = scope.split("/")
    return {
        "kind": "sigv4", "access_key": access_key, "region": region,
        "service": service, "token": None,
    }


def request_view(method: str, url: str, headers: dict[str, str], body: JsonValue) -> JsonValue:
    return {"method": method, "url": url, "auth": authorization_view(headers), "body": body}
