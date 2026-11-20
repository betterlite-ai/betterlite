import asyncio
import inspect
from collections.abc import Awaitable
from typing import Final, Literal, NotRequired, TypedDict
from urllib.parse import parse_qsl, urlsplit, urlunsplit

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.base_llm.containers.transformation import BaseContainerConfig
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.types.containers.main import ContainerListResponse, ContainerObject
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


class Call(TypedDict):
    operation: Literal["create", "list"]
    name: NotRequired[str]
    create_params: NotRequired[dict[str, JsonValue]]
    after: NotRequired[str | None]
    limit: NotRequired[int | None]
    order: NotRequired[str | None]
    api_key: NotRequired[str | None]
    api_base: NotRequired[str | None]
    timeout: NotRequired[int | None]
    status: int
    reply_headers: dict[str, str]
    body: str
    failure: Literal["timeout"] | None


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


async def resolved_object(
    outcome: ContainerObject | Awaitable[ContainerObject],
) -> ContainerObject:
    if inspect.isawaitable(outcome):
        return await outcome
    return outcome


async def resolved_page(
    outcome: ContainerListResponse | Awaitable[ContainerListResponse],
) -> ContainerListResponse:
    if inspect.isawaitable(outcome):
        return await outcome
    return outcome


class Wire:
    def __init__(self, call: Call) -> None:
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


def wanted_headers(sent: httpx.Request) -> dict[str, JsonValue]:
    names = ["authorization", "content-type"] if sent.method == "POST" else ["authorization"]
    return {name.lower(): value for name, value in sent.headers.items() if name.lower() in names}


def timeout_fields(sent: httpx.Request) -> JsonValue:
    if sent.method != "POST":
        return None
    extensions: dict[str, JsonValue] = sent.extensions
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
    query: dict[str, JsonValue] = dict(parse_qsl(parts.query, keep_blank_values=True))
    content = sent.content
    return {
        "method": sent.method,
        "url": urlunsplit((parts.scheme, parts.netloc, parts.path, "", "")),
        "headers": wanted_headers(sent),
        "body": None if content == b"" else JSON.validate_json(content),
        "query": query,
        "timeout": timeout_fields(sent),
    }


def log_entry(call: dict[str, JsonValue]) -> JsonValue:
    args = ARGS.validate_python(call["additional_args"])
    headers: dict[str, JsonValue] = {
        name.lower(): value
        for name, value in HEADERS.validate_python(args.get("headers")).items()
    }
    return {
        "input": call["input"],
        "api_base": args.get("api_base"),
        "headers": headers,
        "body": args.get("complete_input_dict"),
        "params": args.get("params"),
    }


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
    return HTTPHandler(client=httpx.Client(transport=transport))


def answer_fields(wire: Wire, log: RecordingLogging, payload: dict[str, JsonValue]) -> JsonValue:
    return {
        "request": request_fields(wire),
        "log": [log_entry(call) for call in log.calls],
        **payload,
    }


def answer(
    handler: BaseLLMHTTPHandler,
    call: Call,
    config: BaseContainerConfig,
    log: RecordingLogging,
    wire: Wire,
) -> dict[str, JsonValue]:
    litellm_params = GenericLiteLLMParams(
        api_key=call.get("api_key"), api_base=call.get("api_base")
    )
    match call["operation"]:
        case "create":
            timeout = call.get("timeout")
            seconds = 600 if timeout is None else timeout
            name = call.get("name")
            if name is None:
                name = ""
            create_params = call.get("create_params")
            if create_params is None:
                create_params = {}
            created = asyncio.run(
                resolved_object(
                    handler.container_create_handler(  # pyright: ignore[reportUnknownMemberType]  # LiteLLM types the create params as a bare dict, so the handler's type is partially unknown  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/custom_httpx/llm_http_handler.py#L8801-L8888
                        name=name,
                        container_create_request_params=create_params,
                        container_provider_config=config,
                        litellm_params=litellm_params,
                        logging_obj=log,
                        timeout=float(seconds),
                        client=handler_client(wire),
                    )
                )
            )
            container_view = CONTAINER.validate_json(created.model_dump_json())
            return {"response": container_fields(container_view)}
        case "list":
            page = asyncio.run(
                resolved_page(
                    handler.container_list_handler(
                        container_provider_config=config,
                        litellm_params=litellm_params,
                        logging_obj=log,
                        after=call.get("after"),
                        limit=call.get("limit"),
                        order=call.get("order"),
                        client=handler_client(wire),
                    )
                )
            )
            page_view = PAGE.validate_json(page.model_dump_json())
            return {"response": page_fields(page_view)}


def run(input_value: Call) -> JsonValue:
    config = container_config()
    log = RecordingLogging()
    wire = Wire(input_value)
    payload: dict[str, JsonValue]
    try:
        payload = answer(BaseLLMHTTPHandler(), input_value, config, log, wire)
    except BaseLLMException as failure:  # type: ignore[misc]  # LiteLLM's class annotates headers and body as bare dicts, so the type carries Any  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py#L41-L51
        error: dict[str, JsonValue] = {
            "status_code": failure.status_code,
            "message": failure.message,
        }
        payload = {"error": error}
    return answer_fields(wire, log, payload)
