import asyncio
import logging
import os
from collections.abc import Mapping
from datetime import UTC, datetime
from importlib import import_module
from typing import Final, Literal, NotRequired, Protocol, TypedDict

import httpx
import litellm
import pydantic_core
from litellm.llms.base_llm.base_utils import BaseTokenCounter
from litellm.llms.bedrock.common_utils import BedrockModelInfo
from litellm.llms.bedrock.count_tokens import bedrock_token_counter
from litellm.llms.bedrock.count_tokens.bedrock_token_counter import BedrockTokenCounter
from litellm.llms.bedrock.count_tokens.handler import BedrockCountTokensHandler
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.types.utils import TokenCountResponse
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

JSON: Final = TypeAdapter(JsonValue)
CLIENT_KEY: Final = "async_httpx_client" + "bedrock"
PROCESSING_PREFIX: Final = "CountTokens processing error: "
INTERNAL_SERVER_ERROR: Final = 500
REPORTED_HEADERS: Final = (
    "content-type",
    "authorization",
    "x-amz-date",
    "x-amz-security-token",
    "x-amz-content-sha256",
)
DEBUG_SUBJECTS: Final = {
    "Processing CountTokens request for resolved model: ": "processing",
    "Retrieved AWS region: ": "region",
    "Transformed request: ": "transformed",
    "Making request to: ": "endpoint",
    "Response status: ": "status",
    "Bedrock response: ": "reply",
    "Final response: ": "final",
}
ERROR_SUBJECTS: Final = {
    "AWS Bedrock error: ": "AWS Bedrock error",
    "HTTP error in CountTokens handler: ": "HTTP error in CountTokens handler",
    "Error in CountTokens handler: ": "Error in CountTokens handler",
}
API_WARNING: Final = "Bedrock CountTokens API error: status="
GENERIC_WARNING: Final = "Error calling Bedrock CountTokens API: "


class Environment(TypedDict, total=False):
    region_name: str | None
    region: str | None
    session_region: str | None
    runtime_endpoint: str | None
    bearer_token: str | None


class Reply(TypedDict, total=False):
    status: int | None
    body: str | None
    headers: dict[str, str]
    failure: bool


class Call(TypedDict):
    model: str
    now: int
    messages: NotRequired[list[dict[str, JsonValue]] | None]
    contents: NotRequired[list[dict[str, JsonValue]] | None]
    tools: NotRequired[list[dict[str, JsonValue]] | None]
    system: NotRequired[JsonValue]
    request_model: NotRequired[str]
    deployment: NotRequired[dict[str, JsonValue] | None]
    environment: NotRequired[Environment]
    provider: NotRequired[str | None]
    handler: NotRequired[Literal["real", "none", "empty_object"]]
    reply: NotRequired[Reply]


class Calls(TypedDict):
    calls: list[Call]


class Clock:
    def __init__(self) -> None:
        self.moment = datetime.fromtimestamp(0, tz=UTC)
        self.signed = 0

    def __call__(self) -> datetime:
        self.signed += 1
        return self.moment.replace(tzinfo=None)


class Handler(Protocol):
    async def handle_count_tokens_request(
        self,
        request_data: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        resolved_model: str,
    ) -> dict[str, JsonValue] | None: ...


class NoneHandler(Handler):
    @override
    async def handle_count_tokens_request(
        self,
        request_data: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        resolved_model: str,
    ) -> dict[str, JsonValue] | None:
        return None


class EmptyHandler(Handler):
    @override
    async def handle_count_tokens_request(
        self,
        request_data: dict[str, JsonValue],
        litellm_params: dict[str, JsonValue],
        resolved_model: str,
    ) -> dict[str, JsonValue] | None:
        return {}


class NoCounterError(Exception):
    pass


HANDLERS: Final = {
    "real": BedrockCountTokensHandler,
    "none": NoneHandler,
    "empty_object": EmptyHandler,
}


class Provider:
    def __init__(self, reply: Reply) -> None:
        self.reply = reply
        self.first: dict[str, JsonValue] | None = None

    def handle(self, request: httpx.Request) -> httpx.Response:
        if self.first is None:
            timeout = request.extensions["timeout"]  # type: ignore[misc]  # https://github.com/encode/httpx/blob/26d48e0634e6ee9cdc0533996db289ce4b430177/httpx/_models.py#L396
            sent = {name.lower(): value for name, value in request.headers.items()}
            self.first = {
                "method": request.method,
                "url": str(request.url),
                "body": request.content.decode(),
                "timeout": float(timeout["read"]),  # type: ignore[misc]  # https://github.com/encode/httpx/blob/26d48e0634e6ee9cdc0533996db289ce4b430177/httpx/_models.py#L396
                "headers": {name: sent[name] for name in REPORTED_HEADERS if name in sent},
            }
        if self.reply.get("failure"):
            raise httpx.ReadTimeout("fixture transport failure", request=request)
        status = self.reply.get("status")
        body = self.reply.get("body")
        return httpx.Response(
            200 if status is None else status,
            content=("{}" if body is None else body).encode(),
            headers=self.reply.get("headers"),
            request=request,
        )

    def client(self) -> AsyncHTTPHandler:
        handler = AsyncHTTPHandler()
        handler.client = httpx.AsyncClient(transport=httpx.MockTransport(self.handle))
        return handler


def first_argument(record: logging.LogRecord, text: str) -> JsonValue:
    args = record.args
    if not args or not (record.levelno == logging.DEBUG or text.startswith(API_WARNING)):
        return None
    match args:
        case Mapping():
            return JSON.validate_json(pydantic_core.to_json(args))
        case tuple():
            return JSON.validate_json(pydantic_core.to_json(args[0]))


class Events(logging.Filter):
    def __init__(self) -> None:
        super().__init__()
        self.debugs: list[JsonValue] = []
        self.errors: list[JsonValue] = []
        self.warnings: list[JsonValue] = []

    @override
    def filter(self, record: logging.LogRecord) -> bool:
        text = record.getMessage()
        value = first_argument(record, text)
        for prefix, subject in DEBUG_SUBJECTS.items():
            if record.levelno == logging.DEBUG and text.startswith(prefix):
                self.debugs.append({"subject": subject, "value": value})
        for prefix, subject in ERROR_SUBJECTS.items():
            if record.levelno == logging.ERROR and text.startswith(prefix):
                self.errors.append({"subject": subject})
        if record.levelno == logging.WARNING:
            self.warn(text, value)
        return True

    def warn(self, text: str, value: JsonValue) -> None:
        if text.startswith(API_WARNING):
            self.warnings.append({"subject": "Bedrock CountTokens API error", "status": value})
        elif text.startswith(GENERIC_WARNING):
            subject = "Error calling Bedrock CountTokens API"
            self.warnings.append({"subject": subject, "status": None})


def error_message_kind(message: str | None, status: int | None) -> str | None:
    if message is None:
        return None
    if status == INTERNAL_SERVER_ERROR:
        return "processing" if message.startswith(PROCESSING_PREFIX) else "generic"
    return message


def result_json(result: TokenCountResponse) -> JsonValue:
    result.error_message = error_message_kind(result.error_message, result.status_code)
    return JSON.validate_json(result.model_dump_json())


def prepare(call: Call, clock: Clock) -> None:
    environment = call.get("environment")
    fixture: Environment = {} if environment is None else environment
    values = {
        "AWS_REGION_NAME": fixture.get("region_name"),
        "AWS_REGION": fixture.get("region"),
        "AWS_DEFAULT_REGION": fixture.get("session_region"),
        "AWS_BEDROCK_RUNTIME_ENDPOINT": fixture.get("runtime_endpoint"),
        "AWS_BEARER_TOKEN_BEDROCK": fixture.get("bearer_token"),
        "AWS_ACCESS_KEY_ID": "AKIDEXAMPLE",
        "AWS_SECRET_ACCESS_KEY": "fixture-secret-key",
        "AWS_EC2_METADATA_DISABLED": "true",
    }
    os.environ.clear()
    os.environ.update({name: value for name, value in values.items() if value is not None})
    clock.moment = datetime.fromtimestamp(call["now"], tz=UTC)
    clock.signed = 0
    handler = call.get("handler")
    chosen = HANDLERS["real" if handler is None else handler]
    bedrock_token_counter.BedrockCountTokensHandler = chosen  # type: ignore[attr-defined,assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/count_tokens/bedrock_token_counter.py#L75


async def count_once(
    call: Call,
    counter: BaseTokenCounter,
    client: AsyncHTTPHandler,
) -> TokenCountResponse | None:
    litellm.in_memory_llm_clients_cache.set_cache(key=CLIENT_KEY, value=client)  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/llm_caching_handler.py#L58
    request_model = call.get("request_model")
    return await counter.count_tokens(
        model_to_use=call["model"],
        messages=call.get("messages"),
        contents=call.get("contents"),
        deployment=call.get("deployment"),
        request_model="" if request_model is None else request_model,
        tools=call.get("tools"),
        system=call.get("system"),
    )


def one_call(call: Call, clock: Clock, *, factory: bool) -> dict[str, JsonValue]:
    reply = call.get("reply")
    provider = Provider({} if reply is None else reply)
    events = Events()
    prepare(call, clock)
    counter = BedrockModelInfo().get_token_counter() if factory else BedrockTokenCounter()
    if counter is None:
        raise NoCounterError
    selected = counter.should_use_token_counting_api(call.get("provider"))
    logger = logging.getLogger("LiteLLM")
    logger.handlers = [logging.NullHandler()]
    logger.setLevel(logging.DEBUG)
    logger.filters.insert(0, events)
    try:
        result = asyncio.run(count_once(call, counter, provider.client()))
    finally:
        logger.filters.remove(events)
    return {
        "selected": selected,
        "result": None if result is None else result_json(result),
        "request": provider.first,
        "debug": events.debugs,
        "errors": events.errors,
        "warnings": events.warnings,
        "resolved": clock.signed,
    }


def run(input_value: dict[str, Calls]) -> dict[str, JsonValue]:
    botocore_auth = import_module("botocore.auth")
    clock = Clock()
    botocore_auth.get_current_datetime = clock  # type: ignore[attr-defined]  # https://github.com/boto/botocore/blob/a397b28cd8394140abeb4295f14a7b995c3e69c6/botocore/auth.py#L33
    operation, group = next(iter(input_value.items()))
    factory = operation == "counter_factory"
    calls: list[JsonValue] = [one_call(call, clock, factory=factory) for call in group["calls"]]
    return {"calls": calls}
