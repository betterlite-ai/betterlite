import asyncio
import datetime as dt
import socket
import threading
from collections.abc import Coroutine
from dataclasses import dataclass
from typing import Callable, Final, Literal, NotRequired, TypedDict

import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.types.agents import AgentCreateResponse, AgentDeleteResult, AgentVersionsResponse
from openai import APIStatusError
from pydantic import JsonValue, TypeAdapter

JSON: Final = TypeAdapter(JsonValue)
litellm.suppress_debug_info = True


class ReplyInput(TypedDict):
    status: int
    body: str


class Input(TypedDict):
    operation: Literal["get", "delete", "list_versions", "aget", "adelete", "alist_versions"]
    name: str
    api_key: str
    litellm_call_id: str
    reply: ReplyInput
    provider: NotRequired[str | None]
    extra_headers: NotRequired[dict[str, str] | None]
    page_size: NotRequired[int | None]
    page_token: NotRequired[str | None]


LOGGING_KEYS: Final[tuple[str, ...]] = (
    "model", "name", "optional_params", "custom_llm_provider", "litellm_call_id",
    "user", "stream", "messages", "original_response", "additional_args",
)

# the entry points are decorated without annotations, so each is bound to the type the case relies on
GET: Final[Callable[..., AgentCreateResponse]] = litellm.interactions.agents.get
DELETE: Final[Callable[..., AgentDeleteResult]] = litellm.interactions.agents.delete
LIST_VERSIONS: Final[Callable[..., AgentVersionsResponse]] = litellm.interactions.agents.list_versions
AGET: Final[Callable[..., Coroutine[None, None, AgentCreateResponse]]] = litellm.interactions.agents.aget
ADELETE: Final[Callable[..., Coroutine[None, None, AgentDeleteResult]]] = litellm.interactions.agents.adelete
ALIST_VERSIONS: Final[Callable[..., Coroutine[None, None, AgentVersionsResponse]]] = litellm.interactions.agents.alist_versions

CONNECT_TIMEOUT: Final[float] = 5.0


@dataclass
class RequestRecord:
    method: str
    path: str
    headers: dict[str, str]


@dataclass
class Fixture:
    status: int
    body: bytes
    names: tuple[str, ...]
    request: RequestRecord | None = None


def serve(listener: socket.socket, fixture: Fixture) -> None:
    try:
        connection, _peer = listener.accept()
    except OSError:
        return
    received = b""
    while not received.endswith(b"\r\n\r\n"):
        chunk = connection.recv(1)
        if not chunk:
            break
        received += chunk
    lines = received.decode(errors="replace").split("\r\n")
    method, _space, remainder = lines[0].partition(" ")
    path, _rest, _version = remainder.partition(" ")
    headers: dict[str, str] = {}
    for line in lines[1:]:
        name, _colon, value = line.partition(":")
        lowered = name.strip().lower()
        if lowered in fixture.names:
            headers[lowered] = value.strip()
    fixture.request = RequestRecord(method=method, path=path, headers=headers)
    head = (
        f"HTTP/1.1 {fixture.status} fixture\r\n"
        f"content-length: {len(fixture.body)}\r\n"
        "connection: close\r\n\r\n"
    )
    connection.sendall(head.encode() + fixture.body)
    connection.close()


def build_logging(litellm_call_id: str) -> Logging:
    return Logging(
        model="",
        messages=None,
        stream=False,
        call_type="",
        start_time=dt.datetime.now(tz=dt.UTC),
        litellm_call_id=litellm_call_id,
        function_id="",
    )


def measure(input_value: Input, base: str, log: Logging) -> dict[str, JsonValue]:
    arguments: dict[str, JsonValue | Logging] = {
        "name": input_value["name"],
        "custom_llm_provider": input_value.get("provider"),
        "extra_headers": JSON.validate_python(input_value.get("extra_headers")),
        "api_key": input_value["api_key"],
        "api_base": base,
        "litellm_call_id": input_value["litellm_call_id"],
        "litellm_logging_obj": log,
        "page_size": input_value.get("page_size"),
        "page_token": input_value.get("page_token"),
    }
    match input_value["operation"]:
        case "get":
            created = GET(**arguments)
            return {"response": {"id": created.id, "name": created.name}}
        case "delete":
            removed = DELETE(**arguments)
            return {"response": {"name": removed.name, "deleted": removed.deleted}}
        case "list_versions":
            listed = LIST_VERSIONS(**arguments)
            return {"response": JSON.validate_python(listed.model_dump())}
        case "aget":
            created = asyncio.run(AGET(**arguments))
            return {"response": {"id": created.id, "name": created.name}}
        case "adelete":
            removed = asyncio.run(ADELETE(**arguments))
            return {"response": {"name": removed.name, "deleted": removed.deleted}}
        case "alist_versions":
            listed = asyncio.run(ALIST_VERSIONS(**arguments))
            return {"response": JSON.validate_python(listed.model_dump())}


def error_value(error: APIStatusError) -> dict[str, JsonValue]:
    return {
        "status": error.status_code,
        "type": error.type,
        "param": error.param,
        "code": error.code,
    }


def wanted(input_value: Input) -> tuple[str, ...]:
    names: list[str] = ["content-type", "x-goog-api-key"]
    extra = input_value.get("extra_headers")
    if extra is not None:
        for name in extra:
            lowered = name.lower()
            if lowered not in names:
                names.append(lowered)
    return tuple(names)


def request_value(captured: RequestRecord | None) -> JsonValue:
    if captured is None:
        return {"method": None, "path": None, "headers": {}}
    return JSON.validate_python(
        {"method": captured.method, "path": captured.path, "headers": captured.headers}
    )


def logging_value(log: Logging) -> JsonValue:
    details = dict(log.model_call_details)
    return JSON.validate_python({name: details.get(name) for name in LOGGING_KEYS})


def run(input_value: Input) -> JsonValue:
    listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    listener.bind(("127.0.0.1", 0))
    listener.listen(1)
    listener.settimeout(CONNECT_TIMEOUT)
    address: tuple[str, int] = listener.getsockname()
    fixture = Fixture(
        status=input_value["reply"]["status"],
        body=input_value["reply"]["body"].encode(),
        names=wanted(input_value),
    )
    thread = threading.Thread(target=serve, args=(listener, fixture), daemon=True)
    thread.start()
    log = build_logging(input_value["litellm_call_id"])
    try:
        outcome = measure(input_value, f"http://127.0.0.1:{address[1]}", log)
    except APIStatusError as error:
        outcome = {"error": error_value(error)}
    finally:
        listener.close()
    answer: dict[str, JsonValue] = {
        "request": request_value(fixture.request),
        "logging": logging_value(log),
    }
    answer.update(outcome)
    return answer
