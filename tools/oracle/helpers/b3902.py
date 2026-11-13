import os
import sys
from collections.abc import Generator, Mapping, Sequence
from contextlib import contextmanager
from dataclasses import replace
from enum import Enum
from types import MappingProxyType, ModuleType
from typing import TypeAlias, cast
from unittest.mock import patch

import httpx
from pydantic import BaseModel, Field

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
READ_VARIABLES = ("LANGFUSE_MAX_RETRIES", "LANGFUSE_OTEL_TRACES_EXPORT_PATH", "LANGFUSE_TIMEOUT")
UNENCODABLE_KEY = "test.unencodable"
HTTP_ERROR_FLOOR = 400


class _Stub:
    def __init__(self, *args: object, **kwargs: object) -> None:
        del args, kwargs


class _SpanExportResult(Enum):
    SUCCESS = 1
    FAILURE = 2


class _Attributes:
    TRACE_INPUT = "langfuse.trace.input"
    TRACE_OUTPUT = "langfuse.trace.output"
    TRACE_METADATA = "langfuse.trace.metadata"
    OBSERVATION_INPUT = "langfuse.observation.input"
    OBSERVATION_OUTPUT = "langfuse.observation.output"
    OBSERVATION_METADATA = "langfuse.observation.metadata"


class _SpanLimits(_Stub):
    UNSET = -1


class _ReadableSpan:
    # Only the name and attributes are read; the other OTel fields pass through untouched
    _PASSED = ("context", "parent", "resource", "events", "links", "kind", "status",
               "start_time", "end_time", "instrumentation_scope")

    def __init__(self, *, name: str, attributes: Mapping[str, object], **fields: object) -> None:
        self.name = name
        self.attributes = MappingProxyType(dict(attributes))
        for field in self._PASSED:
            setattr(self, field, fields.get(field))


class _Encoded:
    def __init__(self, token: bytes) -> None:
        self.token = token

    def SerializeToString(self) -> bytes:  # noqa: N802 - protobuf's own spelling
        return self.token


class _Encoder:
    def __init__(self) -> None:
        self.bodies: list[list[_ReadableSpan]] = []

    def __call__(self, spans: Sequence[_ReadableSpan]) -> _Encoded:
        batch: list[_ReadableSpan] = list(spans)
        if any(UNENCODABLE_KEY in span.attributes for span in batch):
            raise TypeError("the OTLP encoder rejected a span")
        self.bodies.append(batch)
        return _Encoded(str(len(self.bodies) - 1).encode("ascii"))


class _Reply(BaseModel):
    status: int = 200
    transport_error: bool = False


class _Handler:
    def __init__(self, replies: Sequence[_Reply], encoder: _Encoder) -> None:
        self.replies: list[_Reply] = list(replies)
        self.encoder = encoder
        self.requests: list[Json] = []

    def post(
        self,
        url: str,
        *,
        data: bytes | None = None,
        headers: Mapping[str, str] | None = None,
        timeout: float | None = None,
    ) -> httpx.Response:
        body = b"" if data is None else data
        spans = self.encoder.bodies[int(body.decode("ascii"))]
        given_headers: Mapping[str, str] = {} if headers is None else headers
        self.requests.append(
            {
                "url": url,
                "headers": _headers_json(given_headers),
                "timeout": 0.0 if timeout is None else float(timeout),
                "spans": [_span_json(span) for span in spans],
            }
        )
        reply = self.replies.pop(0)
        request = httpx.Request("POST", url)
        if reply.transport_error:
            raise httpx.ConnectError("the connection failed", request=request)
        response = httpx.Response(reply.status, request=request)
        if reply.status >= HTTP_ERROR_FLOOR:
            raise httpx.HTTPStatusError(f"HTTP {reply.status}", request=request, response=response)
        return response


class _Sleeps:
    def __init__(self) -> None:
        self.delays: list[Json] = []

    def __call__(self, seconds: float) -> None:
        self.delays.append(int(seconds))

    def taken(self) -> list[Json]:
        return list(self.delays)


def _install(name: str, **members: object) -> None:
    module = ModuleType(name)
    module.__path__ = []
    vars(module).update(members)
    sys.modules[name] = module


def _stubs(*names: str) -> dict[str, object]:
    return {name: type(name, (_Stub,), {}) for name in names}


def _install_sdk_stubs() -> None:
    _install("langfuse", LangfuseOtelSpanAttributes=_Attributes)
    _install("langfuse.api", **_stubs("LangfuseAPI", "Prompt", "Prompt_Chat"))
    _install("langfuse.api.core")
    _install("langfuse.api.core.api_error", **_stubs("ApiError"))
    _install("langfuse.api.core.request_options", RequestOptions=_Stub)
    _install(
        "langfuse.model",
        **_stubs("BasePromptClient", "ChatPromptClient", "PromptClient", "TextPromptClient"),
    )
    _install("opentelemetry.sdk")
    _install("opentelemetry.sdk.resources", **_stubs("Resource"))
    _install(
        "opentelemetry.sdk.trace",
        ReadableSpan=_ReadableSpan,
        SpanLimits=_SpanLimits,
        **_stubs("TracerProvider"),
    )
    _install(
        "opentelemetry.sdk.trace.export",
        SpanExportResult=_SpanExportResult,
        **_stubs("BatchSpanProcessor", "SpanExporter"),
    )
    _install("opentelemetry.sdk.trace.id_generator", **_stubs("RandomIdGenerator"))
    _install(
        "opentelemetry.sdk.trace.sampling",
        ALWAYS_ON=_Stub(),
        Decision=_Stub(),
        **_stubs("Sampler", "SamplingResult"),
    )
    _install("opentelemetry.exporter")
    _install("opentelemetry.exporter.otlp")
    _install("opentelemetry.exporter.otlp.proto")
    _install("opentelemetry.exporter.otlp.proto.common")
    _install("opentelemetry.exporter.otlp.proto.common.trace_encoder", encode_spans=_Stub())


_install_sdk_stubs()

from litellm.integrations.langfuse import langfuse_sdk as sdk  # noqa: E402 - needs the stubs above


class _SpanInput(BaseModel):
    name: str
    attributes: list[tuple[str, object]] = Field(default_factory=list)


class _Input(BaseModel):
    public_key: str
    secret_key: str
    base_url: str
    environment: dict[str, str] = Field(default_factory=dict)
    spans: list[_SpanInput] = Field(default_factory=list)
    replies: list[_Reply] = Field(default_factory=list)
    mock_mode: bool = False


@contextmanager
def _exporter_environment(values: Mapping[str, str]) -> Generator[None]:
    previous = {name: os.environ.get(name) for name in READ_VARIABLES}
    for name in READ_VARIABLES:
        os.environ.pop(name, None)
    os.environ.update(values)
    try:
        yield
    finally:
        for name, value in previous.items():
            if value is None:
                os.environ.pop(name, None)
            else:
                os.environ[name] = value


def _kind(value: object) -> str:
    if isinstance(value, bool):
        return "bool"
    if isinstance(value, int):
        return "int"
    if isinstance(value, float):
        return "float"
    if isinstance(value, str):
        return "str"
    raise ValueError("attribute values are text, bool, number or a homogeneous list")


def _attribute_list(value: list[object]) -> list[object]:
    kinds = {_kind(item) for item in value}
    if len(kinds) > 1:
        raise ValueError("attribute lists must hold values of one kind")
    return [_attribute_scalar(item) for item in value]


def _attribute_scalar(value: object) -> object:
    _kind(value)
    if isinstance(value, list):
        return _attribute_list(cast("list[object]", value))
    return value


def _pinned_version(_distribution: str) -> str:
    return "4.15.2"


def _result_text(result: object) -> str:
    return "success" if result is _SpanExportResult.SUCCESS else "failure"


def _json_value(value: object) -> Json:
    if value is None or isinstance(value, (bool, int, float, str)):
        return value
    if isinstance(value, (list, tuple)):
        return [_json_value(item) for item in cast("Sequence[object]", value)]
    raise ValueError(f"unexpected attribute value: {type(value).__name__}")


def _readable_span(item: _SpanInput) -> _ReadableSpan:
    attributes: dict[str, object] = {}
    for key, value in item.attributes:
        attributes[key] = _attribute_scalar(value)
    return _ReadableSpan(name=item.name, attributes=attributes)


def _span_json(span: _ReadableSpan) -> Json:
    return {
        "name": span.name,
        "attributes": [[key, _json_value(value)] for key, value in span.attributes.items()],
    }


def _headers_json(headers: Mapping[str, str]) -> Json:
    ordered = sorted(headers.items(), key=lambda item: item[0].lower())
    return [[name.lower(), value] for name, value in ordered]


def run(input_value: dict[str, Json]) -> Json:
    request = _Input.model_validate(input_value)
    spans = [_readable_span(item) for item in request.spans]
    if request.mock_mode:
        result = cast(
            "object",
            sdk.DiscardingSpanExporter().export(  # pyright: ignore[reportUnknownMemberType]
                spans
            ),
        )
        return {"result": _result_text(result), "requests": [], "sleeps": []}
    encoder = _Encoder()
    handler = _Handler(request.replies, encoder)
    sleeps = _Sleeps()
    with (
        _exporter_environment(request.environment),
        patch.object(sdk, "version", _pinned_version),
        patch.object(sdk, "encode_spans", encoder),
        patch.object(sdk, "sleep", sleeps),
    ):
        built = sdk._build_span_exporter(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            public_key=request.public_key,
            secret_key=request.secret_key,
            base_url=request.base_url,
        )
        exporter = replace(built, handler=handler)
        result = cast(
            "object",
            exporter.export(spans),  # pyright: ignore[reportUnknownMemberType, reportArgumentType]
        )
    observation: dict[str, Json] = {
        "result": _result_text(result),
        "requests": handler.requests,
        "sleeps": sleeps.taken(),
    }
    return observation
