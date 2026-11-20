import asyncio
from collections.abc import Coroutine
from typing import Final, Literal, NotRequired, TypedDict
from urllib.parse import parse_qsl, urlsplit, urlunsplit

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.base_llm.containers.transformation import BaseContainerConfig
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.types.router import GenericLiteLLMParams
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

JSON: Final = TypeAdapter(JsonValue)


class ExpiresView(TypedDict):
    anchor: str
    minutes: int


class ContainerView(TypedDict):
    id: str
    created_at: int
    status: str
    expires_after: ExpiresView | None
    last_active_at: int | None
    name: str | None


class PageView(TypedDict):
    data: list[ContainerView]
    first_id: str | None
    last_id: str | None
    has_more: bool


class TimeoutView(TypedDict):
    connect: float | None
    read: float | None
    write: float | None
    pool: float | None


class CommonCall(TypedDict):
    api_key: str | None
    api_base: str | None
    timeout: int | None
    status: int
    reply_headers: dict[str, str]
    body: str
    failure: Literal["timeout"] | None


class CreateCall(CommonCall):
    operation: Literal["create"]
    name: str
    create_params: dict[str, JsonValue]


class ListCall(CommonCall):
    operation: Literal["list"]
    after: str | None
    limit: int | None
    order: str | None


Call = CreateCall | ListCall


class CallArgs(TypedDict):
    api_base: NotRequired[JsonValue]
    headers: NotRequired[JsonValue]
    complete_input_dict: NotRequired[JsonValue]
    params: NotRequired[JsonValue]


ARGS: Final = TypeAdapter(CallArgs)
CONTAINER: Final = TypeAdapter(ContainerView)
PAGE: Final = TypeAdapter(PageView)
HEADERS: Final = TypeAdapter(dict[str, str])
EXTENSIONS: Final = TypeAdapter(dict[str, JsonValue])
TIMEOUT: Final = TypeAdapter(TimeoutView)


class Wire:
    def __init__(self, call: CommonCall) -> None:
        self.call = call
        self.requests: list[httpx.Request] = []

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(request)
        if self.call["failure"] == "timeout":
            raise httpx.ReadTimeout("the container request ran out of time", request=request)
        return httpx.Response(
            status_code=self.call["status"],
            headers=self.call["reply_headers"],
            content=self.call["body"].encode(),
            request=request,
        )


class RecordingLogging(Logging):
    def __init__(self) -> None:
        self.calls: list[dict[str, JsonValue]] = []

    @override
    def pre_call(self, *args: JsonValue, **values: JsonValue) -> None:
        logged = args[0] if args else values["input"]
        self.calls.append({"input": logged, "additional_args": values["additional_args"]})


def wanted_headers(sent: httpx.Request) -> dict[str, str]:
    names = ["authorization", "content-type"] if sent.method == "POST" else ["authorization"]
    return {name.lower(): value for name, value in sent.headers.items() if name.lower() in names}


def timeout_fields(sent: httpx.Request) -> JsonValue:
    if sent.method != "POST":
        return None
    extensions = EXTENSIONS.validate_python(sent.extensions)
    view = TIMEOUT.validate_python(extensions.get("timeout"))
    read = view["read"]
    return None if read is None else int(read)


def request_fields(wire: Wire) -> JsonValue:
    if not wire.requests:
        return {
            "method": None,
            "url": None,
            "headers": {},
            "body": None,
            "query": {},
            "timeout": None,
        }
    sent = wire.requests[0]
    parts = urlsplit(str(sent.url))
    query: dict[str, JsonValue] = {}
    for name, value in parse_qsl(parts.query, keep_blank_values=True):
        query[name] = value
    headers: dict[str, JsonValue] = {}
    for name, value in wanted_headers(sent).items():
        headers[name] = value
    content = sent.content
    return {
        "method": sent.method,
        "url": urlunsplit((parts.scheme, parts.netloc, parts.path, "", "")),
        "headers": headers,
        "body": None if content == b"" else JSON.validate_json(content),
        "query": query,
        "timeout": timeout_fields(sent),
    }


def log_fields(calls: list[dict[str, JsonValue]]) -> JsonValue:
    entries: list[JsonValue] = []
    for call in calls:
        args = ARGS.validate_python(call["additional_args"])
        headers: dict[str, JsonValue] = {}
        for name, value in HEADERS.validate_python(args.get("headers")).items():
            headers[name.lower()] = value
        entries.append(
            {
                "input": call["input"],
                "api_base": args.get("api_base"),
                "headers": headers,
                "body": args.get("complete_input_dict"),
                "params": args.get("params"),
            }
        )
    return entries


def container_fields(view: ContainerView) -> JsonValue:
    expires = view["expires_after"]
    expiry: JsonValue = None
    if expires is not None:
        expiry = {"anchor": expires["anchor"], "minutes": expires["minutes"]}
    return {
        "id": view["id"],
        "object": "container",
        "created_at": view["created_at"],
        "status": view["status"],
        "expires_after": expiry,
        "last_active_at": view["last_active_at"],
        "name": view["name"],
    }


def page_fields(view: PageView) -> JsonValue:
    return {
        "object": "list",
        "data": [container_fields(item) for item in view["data"]],
        "first_id": view["first_id"],
        "last_id": view["last_id"],
        "has_more": view["has_more"],
    }


def container_config() -> BaseContainerConfig:
    config = ProviderConfigManager.get_provider_container_config(LlmProviders.OPENAI)
    if config is None:
        raise ValueError("openai is not a container provider")
    return config


def handler_client(wire: Wire) -> HTTPHandler:
    transport = httpx.MockTransport(wire.handle)
    return HTTPHandler(client=httpx.Client(transport=transport, timeout=None))


def answer_fields(wire: Wire, log: RecordingLogging, payload: dict[str, JsonValue]) -> JsonValue:
    return {
        "request": request_fields(wire),
        "log": log_fields(log.calls),
        **payload,
    }


def create_answer(call: CreateCall) -> JsonValue:
    config = container_config()
    log = RecordingLogging()
    wire = Wire(call)
    handler = BaseLLMHTTPHandler()
    timeout = 600 if call["timeout"] is None else call["timeout"]
    try:
        outcome = handler.container_create_handler(
            name=call["name"],
            container_create_request_params=call["create_params"],
            container_provider_config=config,
            litellm_params=GenericLiteLLMParams(
                api_key=call["api_key"],
                api_base=call["api_base"],
            ),
            logging_obj=log,
            timeout=float(timeout),
            client=handler_client(wire),
        )
        match outcome:
            case Coroutine() as pending:
                container = asyncio.run(pending)
            case object() as settled:
                container = settled
        view = CONTAINER.validate_json(container.model_dump_json())
    except BaseLLMException as failure:
        error: dict[str, JsonValue] = {
            "status_code": failure.status_code,
            "message": failure.message,
        }
        return answer_fields(wire, log, {"error": error})
    return answer_fields(wire, log, {"response": container_fields(view)})


def list_answer(call: ListCall) -> JsonValue:
    config = container_config()
    log = RecordingLogging()
    wire = Wire(call)
    handler = BaseLLMHTTPHandler()
    try:
        outcome = handler.container_list_handler(
            container_provider_config=config,
            litellm_params=GenericLiteLLMParams(
                api_key=call["api_key"],
                api_base=call["api_base"],
            ),
            logging_obj=log,
            after=call["after"],
            limit=call["limit"],
            order=call["order"],
            client=handler_client(wire),
        )
        match outcome:
            case Coroutine() as pending:
                page = asyncio.run(pending)
            case object() as settled:
                page = settled
        view = PAGE.validate_json(page.model_dump_json())
    except BaseLLMException as failure:
        error: dict[str, JsonValue] = {
            "status_code": failure.status_code,
            "message": failure.message,
        }
        return answer_fields(wire, log, {"error": error})
    return answer_fields(wire, log, {"response": page_fields(view)})


def run(input_value: Call) -> JsonValue:
    if input_value["operation"] == "create":
        return create_answer(input_value)
    return list_answer(input_value)
