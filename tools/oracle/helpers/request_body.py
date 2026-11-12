import asyncio
import base64
from collections.abc import Callable
from typing import Literal, NotRequired, TypeAlias, TypedDict, cast

from litellm.proxy._types import ProxyException
from litellm.proxy.common_utils import http_parsing_utils
from pydantic import JsonValue, TypeAdapter
from starlette.requests import Request
from starlette.types import Message, Scope

Json: TypeAlias = JsonValue

MEGABYTE = 1024 * 1024
BINARY_MEDIA_TYPES = frozenset({"application/x-protobuf", "application/protobuf"})
BASE = "https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/http_parsing_utils.py"


class Body(TypedDict):
    encoding: Literal["utf8", "base64"]
    value: str


class RequestIn(TypedDict):
    method: str
    route_path: str
    body: Body
    headers: NotRequired[dict[str, str]]
    query_params: NotRequired[dict[str, str]]
    cached_body: NotRequired[dict[str, Json]]


class MediaTypesCall(TypedDict):
    function: Literal["normalize_media_types"]
    content_types: list[str]


class BinaryBodyCall(TypedDict):
    function: Literal["parse_binary_body"]
    body: Body


class OtlpCall(TypedDict):
    function: Literal["is_otlp_trace_request"]
    method: str
    route_path: str


class OpaqueAudioCall(TypedDict):
    function: Literal["is_opaque_audio_request"]
    route: str
    content_type: str


class ReadCall(TypedDict):
    function: Literal["read_request_body"]
    request: RequestIn
    max_repair_bytes: int | None


class CachedCall(TypedDict):
    function: Literal["cached_body"]
    request: RequestIn


class SetCachedCall(TypedDict):
    function: Literal["set_cached_body"]
    request: RequestIn
    body: dict[str, Json]


class RewriteCall(TypedDict):
    function: Literal["rewrite_request_model"]
    request_data: dict[str, Json]
    model: str
    request: NotRequired[RequestIn]


class Envelope(TypedDict):
    function: str


ENVELOPE = TypeAdapter(Envelope)
MEDIA_TYPES = TypeAdapter(MediaTypesCall)
BINARY_BODY = TypeAdapter(BinaryBodyCall)
OTLP = TypeAdapter(OtlpCall)
OPAQUE_AUDIO = TypeAdapter(OpaqueAudioCall)
READ = TypeAdapter(ReadCall)
CACHED = TypeAdapter(CachedCall)
SET_CACHED = TypeAdapter(SetCachedCall)
REWRITE = TypeAdapter(RewriteCall)


class BodyReceiver:
    def __init__(self, body: bytes) -> None:
        self.body = body
        self.sent = False
        self.reads = 0

    async def __call__(self) -> Message:
        self.reads += 1
        if self.sent:
            return {"type": "http.disconnect"}
        self.sent = True
        return {"type": "http.request", "body": self.body, "more_body": False}


def header_pairs(headers: dict[str, str] | None) -> list[tuple[bytes, bytes]]:
    if headers is None:
        return []
    return [(name.lower().encode(), value.encode()) for name, value in headers.items()]


def query_string(query_params: dict[str, str] | None) -> bytes:
    if query_params is None:
        return b""
    joined = "&".join(f"{name}={value}" for name, value in query_params.items())
    return joined.encode()


def body_bytes(body: Body) -> bytes:
    if body["encoding"] == "base64":
        return base64.b64decode(body["value"])
    return body["value"].encode()


def build_request(data: RequestIn) -> tuple[Request, BodyReceiver]:
    receiver = BodyReceiver(body_bytes(data["body"]))
    scope: Scope = {
        "type": "http",
        "method": data["method"],
        "path": data["route_path"],
        "query_string": query_string(data.get("query_params")),
        "headers": header_pairs(data.get("headers")),
    }
    request = Request(scope, receiver)
    cached = data.get("cached_body")
    if cached is not None:
        http_parsing_utils._safe_set_request_parsed_body(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/http_parsing_utils.py#L331
            request=request,
            parsed_body=cached,
        )
    return request, receiver


def media_type_of(content_type: str) -> str:
    return http_parsing_utils._normalize_media_type(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/http_parsing_utils.py#L66
        content_type
    )


def media_kind(content_type: str) -> str:
    if http_parsing_utils.is_json_content_type(content_type):
        return "json"
    if http_parsing_utils._is_form_content_type(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/http_parsing_utils.py#L73
        content_type
    ):
        return "form"
    if media_type_of(content_type) in BINARY_MEDIA_TYPES:
        return "binary"
    return "other"


def media_types_call(input_value: Json) -> Json:
    call = MEDIA_TYPES.validate_python(input_value)
    facts: list[Json] = [
        {"media_type": media_type_of(content_type), "kind": media_kind(content_type)}
        for content_type in call["content_types"]
    ]
    return facts


def binary_body_call(input_value: Json) -> Json:
    call = BINARY_BODY.validate_python(input_value)
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/http_parsing_utils.py#L160
        Json,
        http_parsing_utils._parse_binary_body(body_bytes(call["body"])),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/http_parsing_utils.py#L160
    )


def otlp_request(method: str, route_path: str) -> Request:
    scope: Scope = {"type": "http", "method": method, "path": route_path}
    return Request(scope, BodyReceiver(b""))


def otlp_call(input_value: Json) -> Json:
    call = OTLP.validate_python(input_value)
    return http_parsing_utils.is_otlp_trace_request(
        otlp_request(call["method"], call["route_path"])
    )


def opaque_audio_call(input_value: Json) -> Json:
    call = OPAQUE_AUDIO.validate_python(input_value)
    return http_parsing_utils.is_opaque_audio_pass_through_request(
        route=call["route"], content_type=call["content_type"]
    )


def error_facts(error: ProxyException) -> Json:
    return {
        "status": int(error.code),
        "type": error.type,
        "param": error.param,
        "code": error.code,
    }


def cached_body(request: Request) -> dict[str, Json] | None:
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/http_parsing_utils.py#L303
        "dict[str, Json] | None",
        http_parsing_utils._safe_get_request_parsed_body(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/http_parsing_utils.py#L303
            request=request
        ),
    )


def encoded(raw: bytes | None) -> str | None:
    if raw is None:
        return None
    return base64.b64encode(raw).decode()


def read_body(input_value: Json) -> Json:
    call = READ.validate_python(input_value)
    request, receiver = build_request(call["request"])
    megabytes = 0.0 if call["max_repair_bytes"] is None else call["max_repair_bytes"] / MEGABYTE
    http_parsing_utils.MAX_REQUEST_BODY_SIZE_TO_REPAIR_MB = megabytes  # type: ignore[attr-defined, assignment]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/constants.py#L146
    try:
        parsed = asyncio.run(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/http_parsing_utils.py#L175
            http_parsing_utils._read_request_body(  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/http_parsing_utils.py#L175
                request=request
            )
        )
    except ProxyException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L4325
        return error_facts(error)
    body_read = receiver.reads > 0
    cached = cached_body(request)
    raw = asyncio.run(http_parsing_utils.read_raw_json_body(request=request))
    return {
        "body": cast(Json, parsed),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/http_parsing_utils.py#L175
        "body_read": body_read,
        "cached": cached,
        "raw": encoded(raw),
    }


def cached_call(input_value: Json) -> Json:
    call = CACHED.validate_python(input_value)
    request, _receiver = build_request(call["request"])
    return cached_body(request)


def set_cached_call(input_value: Json) -> Json:
    call = SET_CACHED.validate_python(input_value)
    request, _receiver = build_request(call["request"])
    http_parsing_utils._safe_set_request_parsed_body(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/http_parsing_utils.py#L331
        request=request,
        parsed_body=call["body"],
    )
    return cached_body(request)


def rewrite_call(input_value: Json) -> Json:
    call = REWRITE.validate_python(input_value)
    request_data = dict(call["request_data"])
    request_in = call.get("request")
    if request_in is None:
        http_parsing_utils.rewrite_request_model(
            cast("dict[str, object]", request_data),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/http_parsing_utils.py#L343
            None,
            call["model"],
        )
        return {"request_data": request_data, "cached": None}
    request, _receiver = build_request(request_in)
    http_parsing_utils.rewrite_request_model(
        cast("dict[str, object]", request_data),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/http_parsing_utils.py#L343
        request,
        call["model"],
    )
    return {"request_data": request_data, "cached": cached_body(request)}


RUNNERS: dict[str, Callable[[Json], Json]] = {
    "normalize_media_types": media_types_call,
    "parse_binary_body": binary_body_call,
    "is_otlp_trace_request": otlp_call,
    "is_opaque_audio_request": opaque_audio_call,
    "read_request_body": read_body,
    "cached_body": cached_call,
    "set_cached_body": set_cached_call,
    "rewrite_request_model": rewrite_call,
}


def run(input_value: Json) -> Json:
    function = ENVELOPE.validate_python(input_value)["function"]
    runner = RUNNERS[function]
    return runner(input_value)
