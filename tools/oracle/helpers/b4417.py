import asyncio
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
from litellm.proxy._types import CommonProxyErrors
from litellm.secret_managers import hashicorp_secret_manager as vault

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
FIELDS = {"env", "routes", "steps"}
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
    async def async_read_secret(
        self, secret_name: str, optional_params: Object | None = None,
        timeout: float | None = None,  # noqa: ASYNC109 - mirrors LiteLLM's signature
    ) -> str | None: ...

    def sync_read_secret(
        self, secret_name: str, optional_params: Object | None = None, timeout: float | None = None
    ) -> str | None: ...

    async def async_write_secret(
        self, secret_name: str, secret_value: str, description: str | None = None,
        optional_params: Object | None = None,
        timeout: float | None = None,  # noqa: ASYNC109 - mirrors LiteLLM's signature
    ) -> Object: ...

    async def async_delete_secret(
        self, secret_name: str, recovery_window_in_days: int | None = 7,
        optional_params: Object | None = None,
        timeout: float | None = None,  # noqa: ASYNC109 - mirrors LiteLLM's signature
    ) -> Object: ...

    async def async_rotate_secret(
        self, current_secret_name: str, new_secret_name: str, new_secret_value: str,
        optional_params: Object | None = None,
        timeout: float | None = None,  # noqa: ASYNC109 - mirrors LiteLLM's signature
    ) -> Object: ...


@dataclass(frozen=True)
class Call:
    method: str
    url: str
    headers: dict[str, str]
    body: Json


class Transport:
    def __init__(self, routes: list[Json]) -> None:
        self.replies: dict[str, list[Object]] = {}
        for entry in routes:
            fields = expect_object(entry)
            self.replies[expect_text(fields["route"])] = [
                expect_object(reply) for reply in expect_list(fields["replies"])
            ]
        self.requests: list[Json] = []

    def answer(self, call: Call) -> httpx.Response:
        sent: Object = {
            "method": call.method,
            "url": call.url,
            "namespace": call.headers.get("X-Vault-Namespace"),
            "body": call.body,
            "client_certificate": None,
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
        if reply.get("timeout") is True:
            raise httpx.TimeoutException("the request timed out")
        request = httpx.Request(call.method, call.url)
        return httpx.Response(
            expect_integer(reply["status"]),
            text=expect_text(reply["body"]),
            request=request,
        )


class Client:
    def __init__(self, transport: Transport) -> None:
        self.transport = transport

    def get(self, url: str, *, headers: dict[str, str] | None = None) -> httpx.Response:
        return self.transport.answer(Call("GET", url, dict(headers or {}), None))

    def post(
        self, url: str, *, headers: dict[str, str] | None = None, json: Json = None
    ) -> httpx.Response:
        return self.transport.answer(Call("POST", url, dict(headers or {}), json))

    def delete(self, url: str, *, headers: dict[str, str] | None = None) -> httpx.Response:
        return self.transport.answer(Call("DELETE", url, dict(headers or {}), None))


class AsyncClient:
    def __init__(self, transport: Transport) -> None:
        self.transport = transport

    async def get(self, url: str, *, headers: dict[str, str] | None = None) -> httpx.Response:
        return self.transport.answer(Call("GET", url, dict(headers or {}), None))

    async def post(
        self, url: str, *, headers: dict[str, str] | None = None, json: Json = None
    ) -> httpx.Response:
        return self.transport.answer(Call("POST", url, dict(headers or {}), json))

    async def delete(self, url: str, *, headers: dict[str, str] | None = None) -> httpx.Response:
        return self.transport.answer(Call("DELETE", url, dict(headers or {}), None))


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


def expect_number(value: Json) -> float:
    if not isinstance(value, int | float) or isinstance(value, bool):
        raise ValueError("expected a number")
    return float(value)


@contextmanager
def patched_world(env: Object, transport: Transport) -> Generator[None]:
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

    def sync_client(**_options: object) -> Client:
        return Client(transport)

    def async_client(**_options: object) -> AsyncClient:
        return AsyncClient(transport)

    try:
        with ExitStack() as stack:
            stack.enter_context(patch.object(vault, "_get_httpx_client", sync_client))
            stack.enter_context(
                patch.object(vault, "get_async_httpx_client", async_client)
            )
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


def step_options(step: Object) -> tuple[Object | None, float | None]:
    raw_params = step.get("params")
    raw_timeout = step.get("timeout")
    params = None if raw_params is None else expect_object(raw_params)
    timeout = None if raw_timeout is None else expect_number(raw_timeout)
    return params, timeout


def read_outcome(value: str | None) -> Json:
    return {"value": value}


def operation_outcome(result: Object) -> Json:
    if result.get("status") == "error":
        return {"status": "error"}
    return {"body": result}




async def run_step(manager: Manager, step: Object) -> Json:
    operation = expect_text(step["op"])
    params, timeout = step_options(step)
    match operation:
        case "read":
            value = await manager.async_read_secret(
                expect_text(step["name"]), params, timeout
            )
            return read_outcome(value)
        case "sync_read":
            return read_outcome(
                manager.sync_read_secret(expect_text(step["name"]), params, timeout)
            )
        case "write":
            result = await manager.async_write_secret(
                expect_text(step["name"]),
                expect_text(step["value"]),
                None if step.get("description") is None else expect_text(step["description"]),
                params,
                timeout,
            )
            return operation_outcome(result)
        case "delete":
            deleted = await manager.async_delete_secret(
                expect_text(step["name"]), 7, params, timeout
            )
            return {"status": expect_text(deleted.get("status"))}
        case "rotate":
            rotated = await manager.async_rotate_secret(
                expect_text(step["current"]),
                expect_text(step["new"]),
                expect_text(step["value"]),
                params,
                timeout,
            )
            return operation_outcome(rotated)
        case _:
            raise ValueError(f"unknown b4417 step: {operation}")


async def execute(input_value: Object, transport: Transport) -> Json:
    steps: list[Json] = []
    try:
        manager = cast(Manager, vault.HashicorpSecretManager())
    except ValueError:
        return {"init": {"ok": False}, "steps": steps}
    for step in expect_list(input_value["steps"]):
        result = await run_step(manager, expect_object(step))
        steps.append({"result": result, "requests": transport.requests})
        transport.requests = []
    return {"init": {"ok": True}, "steps": steps}


def run(input_value: Object) -> Json:
    if set(input_value) != FIELDS:
        raise ValueError(f"input fields must be {sorted(FIELDS)}")
    transport = Transport(expect_list(input_value["routes"]))
    with patched_world(expect_object(input_value["env"]), transport):
        return asyncio.run(execute(input_value, transport))
