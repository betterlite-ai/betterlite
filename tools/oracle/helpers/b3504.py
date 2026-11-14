import asyncio
import datetime as dt
import inspect
import json
import threading
from collections.abc import Awaitable, Callable
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import ClassVar, Final, Literal, NotRequired, Protocol, TypedDict
from urllib.parse import urlsplit

import litellm
from litellm.interactions import agents as agents_api
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.types.agents import AgentCreateResponse, AgentListResponse
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

JSON: Final = TypeAdapter(JsonValue)
COMPARED_HEADERS: Final[frozenset[str]] = frozenset({"content-type", "x-goog-api-key"})
CALL_TYPES: Final[dict[str, str]] = {
    "create": "create_agent", "acreate": "create_agent",
    "list": "list_agents", "alist": "list_agents",
}


class AgentsApi(Protocol):
    # LiteLLM annotates no **kwargs on these entry points, so the view fixes only the return type
    @property
    def create(self) -> Callable[..., AgentCreateResponse | Awaitable[AgentCreateResponse]]: ...

    @property
    def acreate(self) -> Callable[..., Awaitable[AgentCreateResponse]]: ...

    @property
    def list(self) -> Callable[..., AgentListResponse | Awaitable[AgentListResponse]]: ...

    @property
    def alist(self) -> Callable[..., Awaitable[AgentListResponse]]: ...


AGENTS_API: AgentsApi = agents_api


class Reply(TypedDict):
    status: int
    body: JsonValue


class Call(TypedDict):
    function: Literal["create", "acreate", "list", "alist"]
    reply: Reply
    kwargs: dict[str, JsonValue]
    name: NotRequired[str]
    base_agent: NotRequired[str | None]
    instructions: NotRequired[str | None]
    base_environment: NotRequired[str | dict[str, JsonValue] | None]
    custom_llm_provider: NotRequired[str | None]
    extra_headers: NotRequired[dict[str, str] | None]
    extra_body: NotRequired[dict[str, JsonValue] | None]


class RecordingServer(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, reply: Reply, wanted: frozenset[str]) -> None:
        super().__init__(("127.0.0.1", 0), _Handler)
        self.reply = reply
        self.wanted = wanted
        self.requests: list[JsonValue] = []
        _Handler.recording = self
        self.thread = threading.Thread(target=self.serve_forever, daemon=True)
        self.thread.start()

    @property
    def base(self) -> str:
        return f"http://127.0.0.1:{self.server_port}"

    def close(self) -> None:
        self.shutdown()
        self.server_close()
        self.thread.join()


class _Handler(BaseHTTPRequestHandler):
    recording: ClassVar["RecordingServer | None"] = None

    def do_GET(self) -> None:
        self._record(None)

    def do_POST(self) -> None:
        lengths = self.headers.get_all("content-length")
        raw = self.rfile.read(0 if lengths is None else int(lengths[0]))
        self._record(None if raw == b"" else JSON.validate_json(raw))

    def _record(self, body: JsonValue) -> None:
        server = _Handler.recording
        if server is None:
            raise RuntimeError("the handler is not on a RecordingServer")
        target = urlsplit(self.path)
        lowered = {name.lower(): value for name, value in self.headers.items()}
        kept = {
            name: value
            for name, value in lowered.items()
            if name in COMPARED_HEADERS or name in server.wanted
        }
        server.requests.append(
            {
                "method": self.command,
                "path": target.path,
                "query": target.query,
                "headers": JSON.validate_python(kept),
                "body": body,
            }
        )
        payload = json.dumps(server.reply["body"]).encode("utf-8")
        self.send_response(server.reply["status"])
        self.send_header("content-length", str(len(payload)))
        self.send_header("content-type", "application/json")
        self.send_header("connection", "close")
        self.end_headers()
        self.wfile.write(payload)


async def resolve_created(
    created: AgentCreateResponse | Awaitable[AgentCreateResponse],
) -> AgentCreateResponse:
    if inspect.isawaitable(created):
        return await created
    return created


async def resolve_listed(
    listed: AgentListResponse | Awaitable[AgentListResponse],
) -> AgentListResponse:
    if inspect.isawaitable(listed):
        return await listed
    return listed


def extra_names(call: Call) -> frozenset[str]:
    headers = call.get("extra_headers")
    if headers is None:
        return frozenset()
    return frozenset(name.lower() for name in headers)


def answer(call: Call, logging_obj: Logging, kwargs: dict[str, JsonValue]) -> JsonValue:
    if call["function"] in ("create", "acreate"):
        name = call.get("name")
        if name is None:
            raise ValueError("a create call carries the name of the agent to create")
        if call["function"] == "create":
            created: AgentCreateResponse | Awaitable[AgentCreateResponse] = AGENTS_API.create(
                name=name,
                base_agent=call.get("base_agent"),
                instructions=call.get("instructions"),
                base_environment=call.get("base_environment"),
                custom_llm_provider=call.get("custom_llm_provider"),
                extra_headers=call.get("extra_headers"),
                extra_body=call.get("extra_body"),
                timeout=None,
                litellm_logging_obj=logging_obj,
                **kwargs,
            )
        else:
            created = AGENTS_API.acreate(
                name=name,
                base_agent=call.get("base_agent"),
                instructions=call.get("instructions"),
                base_environment=call.get("base_environment"),
                custom_llm_provider=call.get("custom_llm_provider"),
                extra_headers=call.get("extra_headers"),
                extra_body=call.get("extra_body"),
                timeout=None,
                litellm_logging_obj=logging_obj,
                **kwargs,
            )
        return JSON.validate_json(asyncio.run(resolve_created(created)).model_dump_json())
    if call["function"] == "list":
        listed: AgentListResponse | Awaitable[AgentListResponse] = AGENTS_API.list(
            custom_llm_provider=call.get("custom_llm_provider"),
            extra_headers=call.get("extra_headers"),
            timeout=None,
            litellm_logging_obj=logging_obj,
            **kwargs,
        )
    else:
        listed = AGENTS_API.alist(
            custom_llm_provider=call.get("custom_llm_provider"),
            extra_headers=call.get("extra_headers"),
            timeout=None,
            litellm_logging_obj=logging_obj,
            **kwargs,
        )
    return JSON.validate_json(asyncio.run(resolve_listed(listed)).model_dump_json())


def run(input_value: Call) -> JsonValue:
    server = RecordingServer(input_value["reply"], extra_names(input_value))
    logging_obj = Logging(
        model="fixture",
        messages=None,
        stream=False,
        call_type=CALL_TYPES[input_value["function"]],
        start_time=dt.datetime.now(tz=dt.UTC),
        litellm_call_id="call-1",
        function_id="fn-1",
    )
    kwargs: dict[str, JsonValue] = {
        "api_key": "key-1",
        "api_base": server.base,
        "litellm_call_id": "call-1",
        **input_value["kwargs"],
    }
    try:
        result = answer(input_value, logging_obj, kwargs)
    finally:
        server.close()
    return {"requests": server.requests, "result": result, "model": logging_obj.model}
