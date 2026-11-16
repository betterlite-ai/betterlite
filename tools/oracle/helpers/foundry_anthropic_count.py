import asyncio
import logging
import os
from collections.abc import Awaitable
from typing import Final, Literal, NotRequired, Protocol, TypedDict, TypeAlias, TypeVar, runtime_checkable

import httpx
import litellm
from litellm.caching.llm_caching_handler import LLMClientCache
from litellm.constants import request_timeout as DEFAULT_TIMEOUT
from litellm.llms.anthropic.common_utils import AnthropicError
from litellm.llms.azure_ai.anthropic.count_tokens.handler import (
    AzureAIAnthropicCountTokensHandler,
)
from litellm.llms.azure_ai.anthropic.count_tokens.token_counter import (
    AzureAIAnthropicTokenCounter,
)
from litellm.llms.azure_ai.anthropic.count_tokens.transformation import (
    AzureAIAnthropicCountTokensConfig,
)
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.types.utils import LlmProviders, TokenCountResponse
from pydantic import ConfigDict, JsonValue as Json, TypeAdapter
from typing_extensions import override

CLIENT_KEY: Final = "async_httpx_client"
CACHE_KEY: Final = CLIENT_KEY + LlmProviders.AZURE_AI
CACHE_TTL: Final = 600
HEADERS: Final = (
    "content-type",
    "x-api-key",
    "api-key",
    "authorization",
    "anthropic-version",
    "anthropic-beta",
)
SUCCESS: Final = 200
INTERNAL_SERVER_ERROR: Final = 500
PROCESSING_PREFIX: Final = "CountTokens processing error: "
DECODER: TypeAdapter[Json] = TypeAdapter(Json)
SECONDS: TypeAdapter[dict[str, float]] = TypeAdapter(dict[str, float])
Outcome = TypeVar("Outcome")

Env: TypeAlias = dict[str, str]
Params: TypeAlias = dict[str, Json]
Messages: TypeAlias = list[dict[str, object]]  # ast-grep-ignore: no-loose-types


@runtime_checkable
class ClientCache(Protocol):
    def set_cache(self, key: str, value: AsyncHTTPHandler, *, ttl: int) -> None: ...


CLIENT_CACHE: TypeAdapter[ClientCache] = TypeAdapter(
    ClientCache, config=ConfigDict(arbitrary_types_allowed=True)
)


class Reply(TypedDict):
    status: NotRequired[int]
    body: str


class Globals(TypedDict):
    api_key: NotRequired[str | None]
    azure_key: NotRequired[str | None]


class HandleCase(TypedDict):
    model: str
    messages: Messages
    api_key: str
    api_base: str
    system: NotRequired[Json]
    tools: NotRequired[Messages]
    timeout: NotRequired[float]
    request_timeout: NotRequired[float]
    litellm_params: NotRequired[Params]
    globals: NotRequired[Globals]
    environment: NotRequired[Env]
    reply: NotRequired[Reply]


class CounterCase(TypedDict):
    model: str
    messages: NotRequired[Messages | None]
    contents: NotRequired[Messages | None]
    system: NotRequired[Json]
    tools: NotRequired[Messages]
    request_timeout: NotRequired[float]
    litellm_params: NotRequired[Params]
    globals: NotRequired[Globals]
    environment: NotRequired[Env]
    provider: NotRequired[str]
    request_model: NotRequired[str]
    reply: NotRequired[Reply]


class EndpointCall(TypedDict):
    function: Literal["endpoint"]
    api_bases: list[str]


class HandleCall(TypedDict):
    function: Literal["handle"]
    cases: list[HandleCase]


class CounterCall(TypedDict):
    function: Literal["counter"]
    cases: list[CounterCase]


Input: TypeAlias = EndpointCall | HandleCall | CounterCall


class Sent:
    def __init__(self, reply: Reply | None) -> None:
        self.reply = reply
        self.first: Json = None

    def handle(self, request: httpx.Request) -> httpx.Response:
        if self.first is None:
            headers = {name.lower(): value for name, value in request.headers.items()}
            seconds = SECONDS.validate_python(request.extensions["timeout"])
            body: Json = None if not request.content else DECODER.validate_json(request.content)
            self.first = {
                "url": str(request.url),
                "timeout": seconds["read"],
                "headers": {name: headers.get(name) for name in HEADERS},
                "body": body,
            }
        if self.reply is None:
            raise RuntimeError("fixture transport failure")
        status = self.reply.get("status")
        return httpx.Response(
            SUCCESS if status is None else status,
            content=self.reply["body"].encode(),
            request=request,
        )

    def observed(self) -> Json:
        return self.first


class Warnings(logging.Filter):
    def __init__(self) -> None:
        super().__init__()
        self.entries: list[Json] = []

    @override
    def filter(self, record: logging.LogRecord) -> bool:
        if record.levelno == logging.WARNING:
            self.entries.append(warning_entry(record.getMessage()))
        return True


def warning_entry(message: str) -> Json:
    subject, _separator, detail = message.partition(": ")
    status: Json = None
    if detail.startswith("status="):
        head = detail.removeprefix("status=").split(",", 1)[0]
        try:
            status = int(head)
        except ValueError:
            status = None
    return {"subject": subject, "status": status}


def error_message_kind(message: str | None, status: int | None) -> str | None:
    if message is None:
        return None
    if status == INTERNAL_SERVER_ERROR:
        return "processing" if message.startswith(PROCESSING_PREFIX) else "generic"
    return message


def result_json(result: TokenCountResponse) -> Json:
    result.error_message = error_message_kind(result.error_message, result.status_code)
    return DECODER.validate_json(result.model_dump_json())


def use_environment(values: Env) -> None:
    os.environ.clear()
    os.environ.update(values)


def prepare(case: HandleCase | CounterCase) -> None:
    environment = case.get("environment")
    use_environment({} if environment is None else environment)
    globals = case.get("globals")
    litellm.api_key = None if globals is None else globals.get("api_key")
    litellm.azure_key = None if globals is None else globals.get("azure_key")
    litellm.enable_azure_ad_token_refresh = False
    seconds = case.get("request_timeout")
    litellm.request_timeout = DEFAULT_TIMEOUT if seconds is None else seconds


def install(sent: Sent) -> None:
    holder = LLMClientCache()
    litellm.in_memory_llm_clients_cache = holder
    cache: ClientCache = CLIENT_CACHE.validate_python(holder)
    client = AsyncHTTPHandler(transport=httpx.MockTransport(sent.handle))
    cache.set_cache(key=CACHE_KEY, value=client, ttl=CACHE_TTL)


async def answered(sent: Sent, response: Awaitable[Outcome]) -> Outcome:
    install(sent)
    return await response


def endpoint(call: EndpointCall) -> Json:
    config = AzureAIAnthropicCountTokensConfig()
    urls: list[Json] = [config.get_count_tokens_endpoint(base) for base in call["api_bases"]]
    return {"urls": urls}


def handle_case(case: HandleCase) -> Json:
    prepare(case)
    sent = Sent(case.get("reply"))
    handler = AzureAIAnthropicCountTokensHandler()
    try:
        value = asyncio.run(
            answered(
                sent,
                handler.handle_count_tokens_request(
                    model=case["model"],
                    messages=case["messages"],
                    api_key=case["api_key"],
                    api_base=case["api_base"],
                    litellm_params=case.get("litellm_params"),
                    timeout=case.get("timeout"),
                    tools=case.get("tools"),
                    system=case.get("system"),
                ),
            )
        )
        response: Json = {"value": DECODER.validate_python(value)}
    except AnthropicError as error:
        response = {
            "error": {
                "status_code": error.status_code,
                "processing": error.message.startswith(PROCESSING_PREFIX),
            }
        }
    return {"request": sent.observed(), "response": response}


def counter_case(case: CounterCase) -> Json:
    prepare(case)
    sent = Sent(case.get("reply"))
    counter = AzureAIAnthropicTokenCounter()
    selected = counter.should_use_token_counting_api(case.get("provider"))
    params = case.get("litellm_params")
    deployment: Params | None = None if params is None else {"litellm_params": params}
    request_model = case.get("request_model")
    warnings = Warnings()
    logger = logging.getLogger("LiteLLM")
    logger.handlers = [logging.NullHandler()]
    logger.setLevel(logging.DEBUG)
    logger.filters.insert(0, warnings)
    try:
        result = asyncio.run(
            answered(
                sent,
                counter.count_tokens(
                    model_to_use=case["model"],
                    messages=case.get("messages"),
                    contents=case.get("contents"),
                    deployment=deployment,
                    request_model="" if request_model is None else request_model,
                    tools=case.get("tools"),
                    system=case.get("system"),
                ),
            )
        )
    finally:
        logger.filters.remove(warnings)
    return {
        "selected": selected,
        "result": None if result is None else result_json(result),
        "request": sent.observed(),
        "warnings": warnings.entries,
    }


def run(argument: Input) -> Json:
    match argument["function"]:
        case "endpoint":
            return endpoint(argument)
        case "handle":
            answers: list[Json] = [handle_case(case) for case in argument["cases"]]
            return {"calls": answers}
        case "counter":
            counted: list[Json] = [counter_case(case) for case in argument["cases"]]
            return {"calls": counted}
