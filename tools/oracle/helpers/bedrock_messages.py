import asyncio
import logging
from base64 import b64encode
from collections.abc import AsyncGenerator, Generator, Mapping
from contextlib import contextmanager
from typing import Final, Literal, NotRequired, Protocol, TypeAlias, TypedDict, runtime_checkable

import httpx
import litellm
from litellm.llms.bedrock.common_utils import BedrockError
from litellm.llms.bedrock.messages.invoke_transformations.anthropic_claude3_transformation import (
    AmazonAnthropicClaudeMessagesConfig as MessagesConfig,
)
from litellm.types.router import GenericLiteLLMParams
from pydantic import ConfigDict, JsonValue, TypeAdapter
from typing_extensions import override

from bedrock_invoke_primitives.models import Keys
from bedrock_invoke_primitives.stubs import FixedCredentials, sealed_environment, signing_harness

# an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
litellm.suppress_debug_info = True

Object: TypeAlias = dict[str, JsonValue]
OBJECT: TypeAdapter[Object] = TypeAdapter(Object)
ARBITRARY: ConfigDict = ConfigDict(arbitrary_types_allowed=True)
WARNING_PREFIXES: Final = ("Dropping ", "Bedrock Invoke: ")
REQUEST_BODY: Final[Object] = {"max_tokens": 8}


class InvokeUrl(TypedDict):
    function: Literal["invoke_url"]
    model: str
    credentials: Keys
    now: int
    api_base: NotRequired[str | None]
    stream: NotRequired[bool | None]
    fake_stream: NotRequired[bool | None]
    optional_params: NotRequired[Object]
    headers: NotRequired[dict[str, str]]
    metadata: NotRequired[dict[str, str]]
    spend_logs_metadata: NotRequired[dict[str, str]]
    policy_fields: NotRequired[list[str] | None]
    api_key: NotRequired[str | None]
    configured_bearer_token: NotRequired[str | None]
    request_data: NotRequired[Object]
    environment: NotRequired[dict[str, str]]


class TransformCall(TypedDict):
    model: str
    messages: NotRequired[list[JsonValue]]
    optional_params: NotRequired[Object]
    headers: NotRequired[dict[str, str]]


class Transform(TypedDict):
    function: Literal["transform"]
    model_cost: Object
    calls: list[TransformCall]
    model_info: NotRequired[Object]
    drop_params: NotRequired[bool]


class EventStream(TypedDict):
    function: Literal["event_stream"]
    chunks: list[str]
    response_headers: NotRequired[dict[str, str] | None]


class ErrorClass(TypedDict):
    function: Literal["error_class"]
    message: str
    status_code: int
    headers: NotRequired[dict[str, str]]


class Constructor(TypedDict):
    function: Literal["constructor"]
    kwargs: NotRequired[Object]


Input: TypeAlias = InvokeUrl | Transform | EventStream | ErrorClass | Constructor


class Watch(logging.Handler):
    def __init__(self) -> None:
        super().__init__(logging.DEBUG)
        self.warnings: list[str] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        if record.levelno == logging.WARNING:
            self.warnings.append(record.getMessage())


@contextmanager
def watching() -> Generator[Watch]:
    watch = Watch()
    logger = logging.getLogger("LiteLLM")
    previous_handlers = logger.handlers
    previous_level = logger.level
    logger.handlers = [watch]
    logger.setLevel(logging.DEBUG)
    try:
        yield watch
    finally:
        logger.handlers = previous_handlers
        logger.setLevel(previous_level)


class ErrorView(Protocol):
    status_code: int
    message: str
    headers: Mapping[str, str] | None


class LoggingStub:
    pass


class Frames(httpx.AsyncByteStream):
    def __init__(self, parts: list[bytes]) -> None:
        self.parts = parts

    @override
    async def __aiter__(self) -> AsyncGenerator[bytes, None]:
        for part in self.parts:
            yield part


@runtime_checkable
class MessagesView(Protocol):
    def transform_anthropic_messages_request(  # ast-grep-ignore: no-long-names
        self,
        model: str,
        messages: list[JsonValue],
        params: Object,
        extra: GenericLiteLLMParams,
        headers: dict[str, str],
    ) -> Object: ...

    def validate_anthropic_messages_environment(  # ast-grep-ignore: no-long-names
        self,
        headers: dict[str, str],
        model: str,
        messages: list[JsonValue],
        optional_params: Object,
        litellm_params: Object,
        api_key: str | None = None,
        api_base: str | None = None,
    ) -> tuple[dict[str, str], str | None]: ...

    def get_complete_url(  # ast-grep-ignore: no-long-names
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: Object,
        litellm_params: Object,
        stream: bool | None = None,
    ) -> str: ...

    def sign_request(  # ast-grep-ignore: no-long-names
        self,
        headers: dict[str, str],
        optional_params: Object,
        request_data: Object,
        api_base: str,
        api_key: str | None = None,
        model: str | None = None,
        stream: bool | None = None,
        fake_stream: bool | None = None,
    ) -> tuple[dict[str, str], bytes | None]: ...

    def get_error_class(  # ast-grep-ignore: no-long-names
        self, error_message: str, status_code: int, headers: dict[str, str]
    ) -> ErrorView: ...

    def get_async_streaming_response_iterator(  # ast-grep-ignore: no-long-names
        self,
        model: str,
        response: httpx.Response,
        request_body: Object,
        logging: object,
    ) -> AsyncGenerator[bytes, None]: ...


VIEW: Final[TypeAdapter[MessagesView]] = TypeAdapter(MessagesView, config=ARBITRARY)


def options_of(optional: Object | None) -> Object:
    return {} if optional is None else dict(optional)


def headers_of(headers: dict[str, str] | None) -> dict[str, str]:
    return {} if headers is None else dict(headers)


def metadata_bucket(call: InvokeUrl) -> Object:
    bucket: Object = {}
    metadata = call.get("metadata")
    if metadata is not None:
        bucket.update(metadata)
    spend = call.get("spend_logs_metadata")
    if spend is not None:
        bucket["spend_logs_metadata"] = OBJECT.validate_python(dict(spend))
    return bucket


def invoke_environment(call: InvokeUrl) -> tuple[dict[str, str], str | None]:
    fields = call.get("policy_fields")
    original = litellm.bedrock_request_metadata_fields
    litellm.bedrock_request_metadata_fields = [] if fields is None else list(fields)
    try:
        return VIEW.validate_python(MessagesConfig()).validate_anthropic_messages_environment(
            headers_of(call.get("headers")),
            call["model"],
            [],
            options_of(call.get("optional_params")),
            {"metadata": metadata_bucket(call)},
            call.get("api_key"),
            call.get("api_base"),
        )
    finally:
        litellm.bedrock_request_metadata_fields = original


def signed_request(call: InvokeUrl, url: str, headers: dict[str, str]) -> JsonValue:
    with signing_harness(
        call.get("configured_bearer_token"), FixedCredentials(call["credentials"]), call["now"]
    ):
        signed_headers, body = VIEW.validate_python(MessagesConfig()).sign_request(
            headers=headers,
            optional_params=options_of(call.get("optional_params")),
            request_data=dict(call.get("request_data", {})),
            api_base=url,
            api_key=call.get("api_key"),
            model=call["model"],
            stream=call.get("stream"),
            fake_stream=call.get("fake_stream"),
        )
    return {
        "method": "POST",
        "url": url,
        "headers": OBJECT.validate_python(signed_headers),
        "body": b64encode(b"" if body is None else body).decode(),
    }


def invoke_url(call: InvokeUrl) -> JsonValue:
    headers, api_base = invoke_environment(call)
    environment = call.get("environment")
    with sealed_environment({} if environment is None else dict(environment)):
        url = VIEW.validate_python(MessagesConfig()).get_complete_url(
            call.get("api_base"),
            call.get("api_key"),
            call["model"],
            options_of(call.get("optional_params")),
            {},
            stream=call.get("stream"),
        )
    return {
        "url": {"endpoint_url": url},
        "environment": {"headers": OBJECT.validate_python(headers), "api_base": api_base},
        "signed": signed_request(call, url, headers),
    }


def transform_call(entry: TransformCall) -> JsonValue:
    messages = entry.get("messages")
    with watching() as watch:
        body = VIEW.validate_python(MessagesConfig()).transform_anthropic_messages_request(
            entry["model"],
            [] if messages is None else list(messages),
            options_of(entry.get("optional_params")),
            GenericLiteLLMParams(),
            headers_of(entry.get("headers")),
        )
    warnings: list[JsonValue] = [
        message for message in watch.warnings if message.startswith(WARNING_PREFIXES)
    ]
    return {"body": body, "warnings": warnings}


def transform(call: Transform) -> JsonValue:
    litellm.model_cost = dict(call["model_cost"])
    litellm.drop_params = call.get("drop_params") is True
    return {"calls": [transform_call(entry) for entry in call["calls"]]}


async def drain(call: EventStream) -> JsonValue:
    parts = [bytes.fromhex(chunk) for chunk in call["chunks"]]
    headers = call.get("response_headers")
    response = httpx.Response(
        200, headers={} if headers is None else dict(headers), stream=Frames(parts)
    )
    iterator = VIEW.validate_python(MessagesConfig()).get_async_streaming_response_iterator(
        "fixture-plain", response, REQUEST_BODY, LoggingStub()
    )
    pieces: list[JsonValue] = []
    failure: JsonValue = None
    try:
        async for piece in iterator:
            chunk = piece.hex()
            pieces.append(chunk)
    except BedrockError as error:
        failure = {
            "class": type(error).__name__,
            "message": error.message,
            "status": error.status_code,
        }
    finally:
        await iterator.aclose()
    return {"chunks": pieces, "failure": failure}


def event_stream(call: EventStream) -> JsonValue:
    return asyncio.run(drain(call))


def error_class(call: ErrorClass) -> JsonValue:
    headers = call.get("headers")
    error = VIEW.validate_python(MessagesConfig()).get_error_class(
        call["message"], call["status_code"], {} if headers is None else dict(headers)
    )
    projected: Object = (
        {} if error.headers is None else OBJECT.validate_python(dict(error.headers))
    )
    return {"status": error.status_code, "message": error.message, "headers": projected}


def constructor(call: Constructor) -> JsonValue:
    kwargs = call.get("kwargs")
    MessagesConfig(**{} if kwargs is None else dict(kwargs))
    return {"created": True}


def run(input_value: Input) -> JsonValue:
    match input_value["function"]:
        case "invoke_url":
            return invoke_url(input_value)
        case "transform":
            return transform(input_value)
        case "event_stream":
            return event_stream(input_value)
        case "error_class":
            return error_class(input_value)
        case "constructor":
            return constructor(input_value)
