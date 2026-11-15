import ast
import inspect
import json
import re
from collections.abc import Callable, Iterator, Mapping, Sequence
from functools import partial
from pathlib import Path
from typing import Final, NamedTuple, Protocol

import litellm.proxy.client as litellm_package
import pytest
import requests
from litellm.proxy.client import Client as LiteLLMClient
from proxy_client_support import RecordingServer
from pydantic import JsonValue

import betterlite.proxy.client as package
import betterlite.proxy.client._transport as transport_module
import betterlite.proxy.client.chat as chat_module
import betterlite.proxy.client.client as client_module
import betterlite.proxy.client.exceptions as exceptions_module
import betterlite.proxy.client.health as health_module
import betterlite.proxy.client.model_groups as model_groups_module
import betterlite.proxy.client.models as models_module
import betterlite.proxy.client.users as users_module
from betterlite.proxy.client import (
    ChatClient,
    Client,
    HealthManagementClient,
    HTTPError,
    ModelGroupsManagementClient,
    ModelsManagementClient,
    RequestError,
    UnauthorizedError,
    UsersManagementClient,
)
from betterlite.proxy.client._transport import Request

KEY: Final = "sk-test"
STORED_KEY: Final = "sk-stored"
BASE_URL: Final = "http://localhost:4000"
TIMEOUT: Final = 5
CLIENT_TIMEOUT: Final = 1
SLOW_SECONDS: Final = 1.5
OK: Final = 200
MODEL_BODY: Final = '{"data": [{"id": "model-1"}]}'
CHAT_MODEL: Final = "gpt-4"
HEALTH_PATH: Final = "/health/readiness"
MESSAGES: Final = [{"role": "user", "content": "hello"}]
CLIENT_DIR: Final = Path(__file__).resolve().parents[1] / "python/betterlite/proxy/client"
README_PATH: Final = CLIENT_DIR / "README.md"
RESOURCES: Final = ["chat", "credentials", "http", "keys", "model_groups", "models", "teams"]
MANAGEMENT_RESOURCES: Final = ["credentials", "http", "keys", "model_groups", "models", "teams"]
LITELLM_EXPORTS: Final = [
    "ChatClient",
    "Client",
    "HealthManagementClient",
    "ModelGroupsManagementClient",
    "ModelsManagementClient",
    "UnauthorizedError",
    "UsersManagementClient",
]
EXTRA_EXPORTS: Final = ["HTTPError", "RequestError"]
PYTHON_FENCE: Final = re.compile(r"^```python\n(.*?)^```$", re.MULTILINE | re.DOTALL)

ResourceReply = (
    Mapping[str, JsonValue]
    | Sequence[Mapping[str, JsonValue]]
    | JsonValue
    | requests.Request
    | Request
)


class HTTPLike(Protocol):
    def request(self, method: str, uri: str) -> ResourceReply: ...


class ModelsLike(Protocol):
    def list(self) -> ResourceReply: ...


class ModelGroupsLike(Protocol):
    def info(self) -> ResourceReply: ...


class ChatLike(Protocol):
    def completions(self, model: str, messages: list[dict[str, str]]) -> ResourceReply: ...


class KeysLike(Protocol):
    def list(self) -> ResourceReply: ...


class CredentialsLike(Protocol):
    def list(self) -> ResourceReply: ...


class TeamsLike(Protocol):
    def get_available(self) -> ResourceReply: ...


class ClientLike(Protocol):
    @property
    def http(self) -> HTTPLike: ...

    @property
    def models(self) -> ModelsLike: ...

    @property
    def model_groups(self) -> ModelGroupsLike: ...

    @property
    def chat(self) -> ChatLike: ...

    @property
    def keys(self) -> KeysLike: ...

    @property
    def credentials(self) -> CredentialsLike: ...

    @property
    def teams(self) -> TeamsLike: ...


class ReadmeCall(NamedTuple):
    resource: str
    method: str
    keywords: tuple[str, ...]


@pytest.fixture(autouse=True)
def empty_home(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> Path:
    home = tmp_path / "home"
    home.mkdir()
    monkeypatch.setenv("HOME", str(home))
    return home


@pytest.fixture(autouse=False)
def server() -> Iterator[RecordingServer]:
    with RecordingServer() as recording:
        yield recording


def build_clients(
    base_url: str, api_key: str | None = None, timeout: int = TIMEOUT
) -> tuple[ClientLike, ClientLike]:
    lite: ClientLike = LiteLLMClient(base_url, api_key=api_key, timeout=timeout)
    ours: ClientLike = Client(base_url, api_key=api_key, timeout=timeout)
    return lite, ours


def call_resource(client: ClientLike, resource: str) -> ResourceReply:
    calls: dict[str, Callable[[], ResourceReply]] = {
        "chat": partial(client.chat.completions, CHAT_MODEL, MESSAGES),
        "credentials": client.credentials.list,
        "http": partial(client.http.request, "GET", HEALTH_PATH),
        "keys": client.keys.list,
        "model_groups": client.model_groups.info,
        "models": client.models.list,
        "teams": client.teams.get_available,
    }
    call = calls.get(resource)
    if call is None:
        raise ValueError(f"unknown resource {resource}")
    return call()


def compare_resource_call(
    server: RecordingServer, resource: str, lite: ClientLike, ours: ClientLike
) -> None:
    server.reply(OK, MODEL_BODY)
    call_resource(lite, resource)
    server.reply(OK, MODEL_BODY)
    call_resource(ours, resource)
    assert server.requests[1] == server.requests[0]


def write_token(home: Path, base_url: str, key: str) -> None:
    directory = home / ".litellm"
    directory.mkdir()
    record = {"base_url": base_url, "key": key}
    (directory / "token.json").write_text(json.dumps(record), encoding="utf-8")


def client_signatures(client: Client) -> dict[tuple[str, str], inspect.Signature]:
    return {
        ("chat", "completions"): inspect.signature(client.chat.completions),
        ("chat", "completions_stream"): inspect.signature(client.chat.completions_stream),
        ("credentials", "create"): inspect.signature(client.credentials.create),
        ("credentials", "delete"): inspect.signature(client.credentials.delete),
        ("credentials", "get"): inspect.signature(client.credentials.get),
        ("credentials", "list"): inspect.signature(client.credentials.list),
        ("http", "request"): inspect.signature(client.http.request),
        ("keys", "delete"): inspect.signature(client.keys.delete),
        ("keys", "generate"): inspect.signature(client.keys.generate),
        ("keys", "list"): inspect.signature(client.keys.list),
        ("keys", "update"): inspect.signature(client.keys.update),
        ("model_groups", "info"): inspect.signature(client.model_groups.info),
        ("models", "delete"): inspect.signature(client.models.delete),
        ("models", "list"): inspect.signature(client.models.list),
        ("models", "new"): inspect.signature(client.models.new),
    }


def keyword_problem(method: inspect.Signature, keywords: tuple[str, ...]) -> str:
    for keyword in keywords:
        try:
            method.bind_partial(**{keyword: None})
        except TypeError as error:
            return str(error)
    return ""


def readme_calls(blocks: list[str]) -> Iterator[ReadmeCall]:
    nodes = (node for block in blocks for node in ast.walk(ast.parse(block)))
    for node in nodes:
        match node:
            case ast.Call(
                func=ast.Attribute(
                    value=ast.Attribute(value=ast.Name(id="client"), attr=resource),
                    attr=method,
                ),
                keywords=keywords,
            ):
                names = [keyword.arg for keyword in keywords if keyword.arg is not None]
                yield ReadmeCall(resource, method, tuple(names))
            case ast.AST():
                pass


def test_client_attributes(server: RecordingServer) -> None:
    lite, ours = build_clients(server.url, KEY)
    lite_names = sorted(name for name in dir(lite) if not name.startswith("_"))
    ours_names = sorted(name for name in dir(ours) if not name.startswith("_"))
    assert lite_names == RESOURCES
    assert ours_names == RESOURCES


@pytest.mark.parametrize("resource", RESOURCES)
@pytest.mark.parametrize(("api_key", "expected"), [(KEY, f"Bearer {KEY}"), (None, None)])
def test_client_authorization(
    server: RecordingServer, resource: str, api_key: str | None, expected: str | None
) -> None:
    lite, ours = build_clients(f"{server.url}/", api_key)
    compare_resource_call(server, resource, lite, ours)
    assert server.requests[0].authorization == expected
    assert server.requests[1].authorization == expected


def test_client_stored_login(empty_home: Path, server: RecordingServer) -> None:
    write_token(empty_home, server.url, STORED_KEY)
    lite, ours = build_clients(server.url)
    compare_resource_call(server, "models", lite, ours)
    assert server.requests[0].authorization == f"Bearer {STORED_KEY}"
    assert server.requests[1].authorization == f"Bearer {STORED_KEY}"


def test_client_other_server_login(empty_home: Path, server: RecordingServer) -> None:
    write_token(empty_home, "http://elsewhere:4000", STORED_KEY)
    lite, ours = build_clients(server.url)
    compare_resource_call(server, "models", lite, ours)
    assert server.requests[0].authorization is None
    assert server.requests[1].authorization is None


def test_client_empty_key(empty_home: Path, server: RecordingServer) -> None:
    write_token(empty_home, server.url, STORED_KEY)
    lite, ours = build_clients(server.url, "")
    compare_resource_call(server, "models", lite, ours)
    assert server.requests[0].authorization == f"Bearer {STORED_KEY}"
    assert server.requests[1].authorization == f"Bearer {STORED_KEY}"


@pytest.mark.parametrize("resource", MANAGEMENT_RESOURCES)
def test_client_timeout(server: RecordingServer, resource: str) -> None:
    lite, ours = build_clients(server.url, KEY, CLIENT_TIMEOUT)
    server.reply(OK, MODEL_BODY, delay=SLOW_SECONDS)
    with pytest.raises(requests.exceptions.RequestException):
        call_resource(lite, resource)
    server.reply(OK, MODEL_BODY, delay=SLOW_SECONDS)
    with pytest.raises(RequestError):
        call_resource(ours, resource)


def test_client_chat_timeout(server: RecordingServer) -> None:
    lite, ours = build_clients(server.url, KEY, CLIENT_TIMEOUT)
    chat_reply: JsonValue = {"choices": [{"message": {"content": "hello"}}]}
    body = json.dumps(chat_reply)
    server.reply(OK, body, delay=SLOW_SECONDS)
    lite_reply = call_resource(lite, "chat")
    server.reply(OK, body, delay=SLOW_SECONDS)
    ours_reply = call_resource(ours, "chat")
    assert lite_reply == ours_reply == chat_reply


def test_exports_all() -> None:
    assert sorted(package.__all__) == package.__all__
    ours = [name for name in package.__all__ if name not in EXTRA_EXPORTS]
    assert ours == litellm_package.__all__
    assert len(package.__all__) == len(LITELLM_EXPORTS) + len(EXTRA_EXPORTS)


def test_exports_names() -> None:
    assert ChatClient is chat_module.ChatClient
    assert Client is client_module.Client
    assert HTTPError is transport_module.HTTPError
    assert HealthManagementClient is health_module.HealthManagementClient
    assert ModelGroupsManagementClient is model_groups_module.ModelGroupsManagementClient
    assert ModelsManagementClient is models_module.ModelsManagementClient
    assert RequestError is transport_module.RequestError
    assert UnauthorizedError is exceptions_module.UnauthorizedError
    assert UsersManagementClient is users_module.UsersManagementClient


def test_readme_examples() -> None:
    text = README_PATH.read_text(encoding="utf-8")
    assert "betterlite.proxy.client" in text
    assert "litellm.proxy.client" not in text
    methods = client_signatures(Client(BASE_URL))
    blocks = [match.group(1) for match in PYTHON_FENCE.finditer(text)]
    calls = list(readme_calls(blocks))
    assert calls
    for call in calls:
        name = f"client.{call.resource}.{call.method}"
        method = methods.get((call.resource, call.method))
        assert method is not None, f"the README calls {name}, which Client does not have"
        problem = keyword_problem(method, call.keywords)
        assert not problem, f"{name} does not accept: {problem}"
