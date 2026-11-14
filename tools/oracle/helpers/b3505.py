import asyncio
import datetime as dt
import socket
import socketserver
import threading
from collections.abc import Awaitable, Coroutine
from dataclasses import dataclass
from functools import partial
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Final, Literal, NotRequired, Protocol, TypedDict, Unpack

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.types.agents import AgentCreateResponse, AgentDeleteResult, AgentVersionsResponse
from openai import APIStatusError
from pydantic import JsonValue, TypeAdapter

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
litellm.suppress_debug_info = True


class Reply(TypedDict):
    status: int
    body: str


class Input(TypedDict):
    operation: Literal["get", "delete", "list_versions", "aget", "adelete", "alist_versions"]
    name: str
    api_key: str
    litellm_call_id: str
    reply: Reply
    provider: NotRequired[str | None]
    extra_headers: NotRequired[dict[str, str] | None]
    page_size: NotRequired[int | None]
    page_token: NotRequired[str | None]


class RequestWire(TypedDict):
    method: str | None
    path: str | None
    headers: dict[str, str]


class Extras(TypedDict):
    api_key: str
    api_base: str
    litellm_call_id: str
    litellm_logging_obj: Logging
    page_size: NotRequired[int | None]
    page_token: NotRequired[str | None]


# the agents SDK's public entry points: a sync one answers its model, async ones a coroutine
class AgentsItemSdk(Protocol):
    def get(
        self,
        name: str,
        custom_llm_provider: str | None = None,
        extra_headers: dict[str, str] | None = None,
        timeout: float | httpx.Timeout | None = None,
        **extras: Unpack[Extras],
    ) -> AgentCreateResponse | Awaitable[AgentCreateResponse]: ...

    def aget(
        self,
        name: str,
        custom_llm_provider: str | None = None,
        extra_headers: dict[str, str] | None = None,
        timeout: float | httpx.Timeout | None = None,
        **extras: Unpack[Extras],
    ) -> Coroutine[None, None, AgentCreateResponse]: ...

    def delete(
        self,
        name: str,
        custom_llm_provider: str | None = None,
        extra_headers: dict[str, str] | None = None,
        timeout: float | httpx.Timeout | None = None,
        **extras: Unpack[Extras],
    ) -> AgentDeleteResult | Awaitable[AgentDeleteResult]: ...

    def adelete(
        self,
        name: str,
        custom_llm_provider: str | None = None,
        extra_headers: dict[str, str] | None = None,
        timeout: float | httpx.Timeout | None = None,
        **extras: Unpack[Extras],
    ) -> Coroutine[None, None, AgentDeleteResult]: ...

    def list_versions(
        self,
        name: str,
        custom_llm_provider: str | None = None,
        extra_headers: dict[str, str] | None = None,
        timeout: float | httpx.Timeout | None = None,
        **extras: Unpack[Extras],
    ) -> AgentVersionsResponse | Awaitable[AgentVersionsResponse]: ...

    def alist_versions(
        self,
        name: str,
        custom_llm_provider: str | None = None,
        extra_headers: dict[str, str] | None = None,
        timeout: float | httpx.Timeout | None = None,
        **extras: Unpack[Extras],
    ) -> Coroutine[None, None, AgentVersionsResponse]: ...


LOGGING_KEYS: Final[tuple[str, ...]] = (
    "model",
    "name",
    "optional_params",
    "custom_llm_provider",
    "litellm_call_id",
    "user",
    "stream",
    "messages",
    "original_response",
    "additional_args",
)


@dataclass
class Captured:
    method: str
    path: str
    headers: dict[str, str]


@dataclass
class Fixture:
    status: int
    body: bytes
    names: tuple[str, ...]
    captured: Captured | None = None


class Answering(BaseHTTPRequestHandler):
    fixture: Fixture

    def __init__(
        self,
        fixture: Fixture,
        request: socket.socket,
        client_address: tuple[str, int],
        server: socketserver.BaseServer,
    ) -> None:
        self.fixture = fixture
        super().__init__(request, client_address, server)

    def do_GET(self) -> None:
        self.answer()

    def do_DELETE(self) -> None:
        self.answer()

    def answer(self) -> None:
        fixture = self.fixture
        fixture.captured = Captured(
            method=self.command,
            path=self.path,
            headers={
                name.lower(): value
                for name, value in self.headers.items()
                if name.lower() in fixture.names
            },
        )
        self.send_response(fixture.status)
        self.send_header("Content-Length", str(len(fixture.body)))
        self.end_headers()
        _written = self.wfile.write(fixture.body)


def as_created(
    result: AgentCreateResponse | Awaitable[AgentCreateResponse],
) -> AgentCreateResponse:
    match result:
        case AgentCreateResponse():
            return result
        case Awaitable():
            raise ValueError("the sync get returned a coroutine")


def as_removed(result: AgentDeleteResult | Awaitable[AgentDeleteResult]) -> AgentDeleteResult:
    match result:
        case AgentDeleteResult():
            return result
        case Awaitable():
            raise ValueError("the sync delete returned a coroutine")


def as_versions(
    result: AgentVersionsResponse | Awaitable[AgentVersionsResponse],
) -> AgentVersionsResponse:
    match result:
        case AgentVersionsResponse():
            return result
        case Awaitable():
            raise ValueError("the sync list_versions returned a coroutine")


def measure(sdk: AgentsItemSdk, input_value: Input, extras: Extras) -> dict[str, JsonValue]:
    name = input_value["name"]
    provider = input_value.get("provider")
    headers = input_value.get("extra_headers")
    match input_value["operation"]:
        case "get":
            # a name keeps the defaulted dict-lookup rule from matching the agents read
            get_agent = sdk.get
            agent = as_created(get_agent(name, provider, headers, **extras))
            return {"response": {"id": agent.id, "name": agent.name}}
        case "delete":
            removed = as_removed(sdk.delete(name, provider, headers, **extras))
            return {"response": {"name": removed.name, "deleted": removed.deleted}}
        case "aget":
            agent = asyncio.run(sdk.aget(name, provider, headers, **extras))
            return {"response": {"id": agent.id, "name": agent.name}}
        case "adelete":
            removed = asyncio.run(sdk.adelete(name, provider, headers, **extras))
            return {"response": {"name": removed.name, "deleted": removed.deleted}}
        case "list_versions":
            extras["page_size"] = input_value.get("page_size")
            extras["page_token"] = input_value.get("page_token")
            versions = as_versions(sdk.list_versions(name, provider, headers, **extras))
            return {"response": JSON.validate_json(versions.model_dump_json())}
        case "alist_versions":
            extras["page_size"] = input_value.get("page_size")
            extras["page_token"] = input_value.get("page_token")
            versions = asyncio.run(sdk.alist_versions(name, provider, headers, **extras))
            return {"response": JSON.validate_json(versions.model_dump_json())}


def wanted(input_value: Input) -> tuple[str, ...]:
    names: list[str] = ["content-type", "x-goog-api-key"]
    extra = input_value.get("extra_headers")
    if extra is not None:
        for name in extra:
            lowered = name.lower()
            if lowered not in names:
                names.append(lowered)
    return tuple(names)


def run(input_value: Input) -> JsonValue:
    fixture = Fixture(
        status=input_value["reply"]["status"],
        body=input_value["reply"]["body"].encode(),
        names=wanted(input_value),
    )
    caller = input_value["litellm_call_id"]
    log = Logging(
        model="",
        messages=None,
        stream=False,
        call_type="",
        start_time=dt.datetime.now(tz=dt.UTC),
        litellm_call_id=caller,
        function_id="",
    )
    outcome: dict[str, JsonValue] = {}
    server = ThreadingHTTPServer(("127.0.0.1", 0), partial(Answering, fixture))
    thread = threading.Thread(target=partial(server.serve_forever, poll_interval=0.01), daemon=True)
    thread.start()
    try:
        extras = Extras(
            api_key=input_value["api_key"],
            api_base=f"http://127.0.0.1:{server.server_address[1]}",
            litellm_call_id=caller,
            litellm_logging_obj=log,
        )
        outcome.update(measure(litellm.interactions.agents, input_value, extras))
    except APIStatusError as error:
        outcome["error"] = {
            "status": error.status_code,
            "type": error.type,
            "param": error.param,
            "code": error.code,
        }
    finally:
        server.shutdown()
        server.server_close()
    captured = fixture.captured
    if captured is None:
        outcome["request"] = JSON.validate_python(RequestWire(method=None, path=None, headers={}))
    else:
        outcome["request"] = JSON.validate_python(
            RequestWire(method=captured.method, path=captured.path, headers=captured.headers)
        )
    details: dict[str, JsonValue] = log.model_call_details
    outcome["logging"] = {name: details.get(name) for name in LOGGING_KEYS}
    return outcome
