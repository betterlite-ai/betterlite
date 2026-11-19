import logging
from collections.abc import AsyncGenerator, Generator
from contextlib import aclosing, contextmanager

import httpx
import litellm
from litellm.llms.bedrock.common_utils import BedrockError
from litellm.llms.bedrock.messages.invoke_transformations.anthropic_claude3_transformation import (
    AmazonAnthropicClaudeMessagesConfig as MessagesConfig,
)
from litellm.types.router import GenericLiteLLMParams
from pydantic import JsonValue
from typing_extensions import override

from .views import (
    REQUEST_BODY,
    WARNING_PREFIXES,
    Constructor,
    ErrorClass,
    EventStream,
    LoggingStub,
    Object,
    Probe,
    Transform,
    TransformCall,
)


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


class Frames(httpx.AsyncByteStream):
    def __init__(self, parts: list[bytes]) -> None:
        self.parts = parts

    @override
    async def __aiter__(self) -> AsyncGenerator[bytes, None]:
        for part in self.parts:
            yield part


def transform_call(entry: TransformCall) -> JsonValue:
    messages = entry.get("messages")
    optional = entry.get("optional_params")
    headers = entry.get("headers")
    with watching() as watch:
        body = Probe().view().transform_anthropic_messages_request(
            entry["model"],
            [] if messages is None else messages,
            {} if optional is None else optional,
            GenericLiteLLMParams(),
            {} if headers is None else headers,
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
        200, headers={} if headers is None else headers, stream=Frames(parts)
    )
    iterator = Probe().view().get_async_streaming_response_iterator(
        "fixture-plain", response, REQUEST_BODY, LoggingStub()
    )
    pieces: list[JsonValue] = []
    failure: JsonValue = None
    try:
        async with aclosing(iterator):
            pieces.extend([piece.hex() async for piece in iterator])
    except BedrockError as error:
        failure = {
            "class": type(error).__name__,
            "message": error.message,
            "status": error.status_code,
        }
    return {"chunks": pieces, "failure": failure}


def error_class(call: ErrorClass) -> JsonValue:
    headers = call.get("headers")
    error = Probe().view().get_error_class(
        call["message"], call["status_code"], {} if headers is None else headers
    )
    projected: Object = {}
    if error.headers is not None:
        projected = error.headers
    return {"status": error.status_code, "message": error.message, "headers": projected}


def constructor(call: Constructor) -> JsonValue:
    kwargs = call.get("kwargs")
    MessagesConfig(**{} if kwargs is None else kwargs)  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/messages/invoke_transformations/anthropic_claude3_transformation.py#L99-L101
    return {"created": True}
