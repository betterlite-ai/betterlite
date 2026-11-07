import asyncio
import json
import os
import sys
from collections.abc import Generator
from contextlib import ExitStack, contextmanager
from dataclasses import dataclass
from types import ModuleType
from typing import Protocol, TypeAlias, cast
from unittest.mock import patch

import httpx
import litellm
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.proxy._types import CommonProxyErrors
from litellm.secret_managers import hashicorp_secret_manager as vault

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
FIELDS = {"env", "routes", "steps"}
START = 1_000_000.0
VARIABLES = (
    "HCP_VAULT_ADDR",
    "HCP_VAULT_NAMESPACE",
    "HCP_VAULT_LOGIN_NAMESPACE",
    "HCP_VAULT_SECRET_NAMESPACE",
    "HCP_VAULT_MOUNT_NAME",
    "HCP_VAULT_PATH_PREFIX",
    "HCP_VAULT_TOKEN",
    "HCP_VAULT_APPROLE_ROLE_ID",
    "HCP_VAULT_APPROLE_SECRET_ID",
    "HCP_VAULT_APPROLE_MOUNT_PATH",
    "HCP_VAULT_CLIENT_CERT",
    "HCP_VAULT_CLIENT_KEY",
    "HCP_VAULT_CERT_ROLE",
    "HCP_VAULT_REFRESH_INTERVAL",
)


class Manager(Protocol):
    @property
    def vault_login_namespace(self) -> str | None: ...

    @property
    def vault_secret_namespace(self) -> str | None: ...

    def _get_request_headers(self) -> dict[str, str]: ...

    def _build_secret_target(self, secret_name: str, optional_params: Object | None) -> Object: ...

    def _get_secret_value_from_json_response(
        self, json_resp: Object | None, data_key: str
    ) -> str | None: ...


class Clock:
    def __init__(self) -> None:
        self.now = START

    def read(self) -> float:
        return self.now


@dataclass(frozen=True)
class Call:
    method: str
    url: str
    headers: dict[str, str]
    body: Json


class Client:
    def __init__(self, transport: "Transport", cert: tuple[str, str] | None) -> None:
        self.transport = transport
        self.cert = cert

    def post(
        self, url: str, *, headers: dict[str, str] | None = None, json: Json = None
    ) -> httpx.Response:
        return self.transport.answer(self, Call("POST", url, dict(headers or {}), json))


class Transport:
    def __init__(self, routes: list[Json]) -> None:
        self.replies: dict[str, list[Object]] = {}
        for entry in routes:
            fields = expect_object(entry)
            self.replies[expect_text(fields["route"])] = [
                expect_object(reply) for reply in expect_list(fields["replies"])
            ]
        self.requests: list[Json] = []

    def answer(self, client: Client, call: Call) -> httpx.Response:
        sent: Object = {
            "method": call.method,
            "url": call.url,
            "namespace": call.headers.get("X-Vault-Namespace"),
            "body": call.body,
            "client_certificate": None
            if client.cert is None
            else {"certificate": client.cert[0], "key": client.cert[1]},
        }
        self.requests.append(sent)
        replies = self.replies.get(f"{call.method} {call.url}", [])
        reply = (
            replies.pop(0)
            if len(replies) > 1
            else (replies[0] if replies else {"status": 500, "body": ""})
        )
        if reply.get("failure") is True:
            raise httpx.ConnectError("connection refused")
        request = httpx.Request(call.method, call.url)
        return httpx.Response(
            expect_integer(reply["status"]),
            text=expect_text(reply["body"]),
            request=request,
        )

    def take(self) -> list[Json]:
        taken = self.requests
        self.requests = []
        return taken


def expect_object(value: Json) -> Object:
    if not isinstance(value, dict):
        raise ValueError("expected an object")
    return value


def expect_list(value: Json) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError("expected a list")
    return value


def expect_text(value: Json) -> str:
    if not isinstance(value, str):
        raise ValueError("expected text")
    return value


def expect_integer(value: Json) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValueError("expected an integer")
    return value


@contextmanager
def patched_world(
    env: Object, transport: Transport, clock: Clock
) -> Generator[None]:
    proxy_server = ModuleType("litellm.proxy.proxy_server")
    namespace = cast(dict[str, object], vars(proxy_server))
    namespace["premium_user"] = True
    namespace["CommonProxyErrors"] = CommonProxyErrors
    saved_proxy_server = sys.modules.get(proxy_server.__name__)
    saved_environment = {
        name: value for name, value in os.environ.items() if name in VARIABLES
    }
    saved_client = litellm.secret_manager_client
    saved_system = litellm._key_management_system  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    for name in VARIABLES:
        os.environ.pop(name, None)
    for name, value in env.items():
        os.environ[name] = expect_text(value)
    sys.modules[proxy_server.__name__] = proxy_server

    def approle_client() -> Client:
        return Client(transport=transport, cert=None)

    def certificate_client(*, cert: tuple[str, str], **_options: object) -> Client:
        return Client(transport=transport, cert=cert)

    def clocked_cache(*, default_ttl: int) -> InMemoryCache:
        return InMemoryCache(default_ttl=default_ttl, clock=clock.read)

    try:
        with ExitStack() as stack:
            stack.enter_context(patch.object(vault, "_get_httpx_client", approle_client))
            stack.enter_context(patch.object(httpx, "Client", certificate_client))
            stack.enter_context(patch.object(vault, "InMemoryCache", clocked_cache))
            yield
    finally:
        if saved_proxy_server is None:
            del sys.modules[proxy_server.__name__]
        else:
            sys.modules[proxy_server.__name__] = saved_proxy_server
        for name in VARIABLES:
            os.environ.pop(name, None)
        os.environ.update(saved_environment)
        litellm.secret_manager_client = saved_client
        litellm._key_management_system = saved_system  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]


def init_tag(error: ValueError) -> str:
    if str(error).startswith("Missing Vault authentication credentials"):
        return "missing_credentials"
    return "invalid_refresh_interval"


def token_result(manager: Manager) -> Json:
    try:
        headers = manager._get_request_headers()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    except Exception:  # noqa: BLE001 - LiteLLM wraps every login failure in RuntimeError
        return {"error": "login_failed"}
    return {"token": headers["X-Vault-Token"]}


def target_result(manager: Manager, step: Object) -> Json:
    name = expect_text(step["name"])
    raw_params = step.get("params")
    params = None if raw_params is None else expect_object(raw_params)
    try:
        target = manager._build_secret_target(name, params)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    except ValueError:
        return {"error": "unsafe_secret_name"}
    return {
        "url": target["url"],
        "data_key": target["data_key"],
        "secret_name": target["secret_name"],
    }


def value_result(manager: Manager, step: Object) -> Json:
    body = expect_text(step["body"])
    data_key = expect_text(step["data_key"])
    try:
        parsed = cast(Json, json.loads(body))
    except json.JSONDecodeError:
        return {"value": None}
    document = parsed if isinstance(parsed, dict) else None
    value = manager._get_secret_value_from_json_response(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        document, data_key
    )
    return {"value": value}


async def run_step(manager: Manager, step: Object, clock: Clock) -> Json:
    operation = expect_text(step["op"])
    match operation:
        case "token" | "token_blocking":
            return token_result(manager)
        case "namespaces":
            return {
                "login": manager.vault_login_namespace,
                "secret": manager.vault_secret_namespace,
            }
        case "target":
            return target_result(manager, step)
        case "value":
            return value_result(manager, step)
        case "advance":
            clock.now += expect_integer(step["ms"]) / 1000
            return None
        case _:
            raise ValueError(f"unknown b4416 step: {operation}")


async def execute(input_value: Object, clock: Clock, transport: Transport) -> Json:
    steps: list[Json] = []
    try:
        manager = cast(Manager, vault.HashicorpSecretManager())
    except ValueError as error:
        return {"init": {"ok": False, "error": init_tag(error)}, "steps": steps}
    for step in expect_list(input_value["steps"]):
        result = await run_step(manager, expect_object(step), clock)
        steps.append({"result": result, "requests": transport.take()})
    return {"init": {"ok": True}, "steps": steps}


def run(input_value: Object) -> Json:
    if set(input_value) != FIELDS:
        raise ValueError(f"input fields must be {sorted(FIELDS)}")
    clock = Clock()
    transport = Transport(expect_list(input_value["routes"]))
    with patched_world(expect_object(input_value["env"]), transport, clock):
        return asyncio.run(execute(input_value, clock, transport))
