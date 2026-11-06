import asyncio
import logging
import os
import sys
from collections.abc import Generator
from contextlib import ExitStack, contextmanager
from dataclasses import dataclass
from types import ModuleType, SimpleNamespace
from typing import Protocol, TypeAlias, cast
from unittest.mock import patch

import httpx
import litellm
from litellm._logging import verbose_logger  # pyright: ignore[reportPrivateUsage]
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.proxy._types import CommonProxyErrors
from litellm.secret_managers import cyberark_secret_manager as cyberark

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
FIELDS = {"env", "routes", "steps"}
START = 1_000_000.0
EVENTS = (
    ("Authenticating with CyberArk Conjur", "authenticating"),
    ("Successfully authenticated", "authenticated"),
    ("SSL verification is disabled", "ssl_verification_disabled"),
    ("Created policy entry", "policy_created"),
    ("policy was rejected as unprocessable", "policy_unprocessable"),
    ("Could not ensure variable exists", "policy_refused"),
    ("Error ensuring variable exists", "policy_failed"),
    ("not found in CyberArk Conjur", "secret_not_found"),
    ("Error reading secret from CyberArk Conjur", "read_failed"),
    ("Error writing secret to CyberArk Conjur", "write_failed"),
    ("does not support direct secret deletion", "delete_not_supported"),
)


class Manager(Protocol):
    async def async_read_secret(self, secret_name: str) -> str | None: ...

    def sync_read_secret(self, secret_name: str) -> str | None: ...

    async def async_write_secret(self, secret_name: str, secret_value: str) -> dict[str, str]: ...

    async def async_delete_secret(self, secret_name: str) -> dict[str, str]: ...

    async def async_rotate_secret(
        self, current_secret_name: str, new_secret_name: str, new_secret_value: str
    ) -> dict[str, str]: ...


class Clock:
    def __init__(self) -> None:
        self.now = START

    def read(self) -> float:
        return self.now

    def elapsed_ms(self) -> int:
        return round((self.now - START) * 1000)

    async def sleep(self, seconds: float) -> None:
        self.now += seconds


@dataclass(frozen=True)
class Call:
    method: str
    url: str
    headers: dict[str, str] | None
    content: str | None


@dataclass(frozen=True, kw_only=True)
class Client:
    transport: "Transport"
    verify: bool
    cert: tuple[str, str] | None
    blocking: bool

    def post(
        self, url: str, *, headers: dict[str, str] | None = None, content: str | None = None
    ) -> httpx.Response:
        return self.transport.answer(self, Call("POST", url, headers, content))

    def get(self, url: str, *, headers: dict[str, str] | None = None) -> httpx.Response:
        return self.transport.answer(self, Call("GET", url, headers, None))

    async def apost(
        self, url: str, *, headers: dict[str, str] | None = None, content: str | None = None
    ) -> httpx.Response:
        return self.post(url, headers=headers, content=content)

    async def aget(self, url: str, *, headers: dict[str, str] | None = None) -> httpx.Response:
        return self.get(url, headers=headers)


class Transport:
    def __init__(self, routes: list[Json], clock: Clock) -> None:
        self.replies: dict[str, list[Object]] = {}
        for entry in routes:
            fields = as_object(entry)
            self.replies[as_text(fields["route"])] = [
                as_object(reply) for reply in as_list(fields["replies"])
            ]
        self.clock = clock
        self.requests: list[Json] = []

    def answer(self, client: Client, call: Call) -> httpx.Response:
        route = f"{call.method} {call.url}"
        sent = call.headers or {}
        self.requests.append(
            {
                "route": route,
                "authorization": sent.get("Authorization"),
                "content_type": sent.get("Content-Type"),
                "body": call.content,
                "verify_tls": client.verify,
                "client_certificate": None
                if client.cert is None
                else {"certificate": client.cert[0], "key": client.cert[1]},
                "blocking": None if call.url.endswith("/authenticate") else client.blocking,
                "at_ms": self.clock.elapsed_ms(),
            }
        )
        replies = self.replies.get(route, [])
        reply = (
            replies.pop(0)
            if len(replies) > 1
            else (replies[0] if replies else {"status": 500, "body": ""})
        )
        if reply.get("failure") is True:
            raise httpx.ConnectError("connection refused")
        request = httpx.Request(call.method, call.url)
        return httpx.Response(as_int(reply["status"]), text=as_text(reply["body"]), request=request)


class LogCapture(logging.Handler):
    def __init__(self) -> None:
        super().__init__(logging.DEBUG)
        self.events: list[Json] = []

    def emit(self, record: logging.LogRecord) -> None:
        message = record.getMessage()
        event = next((name for fragment, name in EVENTS if fragment in message), "unrecognized")
        self.events.append({"level": record.levelname.lower(), "event": event})

    def take(self) -> list[Json]:
        taken = self.events
        self.events = []
        # LiteLLM's base rotation logs an AWS-worded error that betterlite does not
        return [entry for entry in taken if as_object(entry)["event"] != "unrecognized"]


def as_object(value: Json) -> Object:
    if not isinstance(value, dict):
        raise ValueError("expected an object")
    return value


def as_list(value: Json) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError("expected a list")
    return value


def as_text(value: Json) -> str:
    if not isinstance(value, str):
        raise ValueError("expected a string")
    return value


def as_int(value: Json) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValueError("expected an integer")
    return value


def ssl_verify(options: dict[str, Json]) -> bool:
    verify = as_object(options["params"])["ssl_verify"]
    if not isinstance(verify, bool):
        raise ValueError("ssl_verify must be a boolean")
    return verify


@contextmanager
def patched_world(env: Object, transport: Transport, clock: Clock) -> Generator[None]:
    proxy_server = ModuleType("litellm.proxy.proxy_server")
    namespace = cast(dict[str, object], vars(proxy_server))
    namespace["premium_user"] = True
    namespace["CommonProxyErrors"] = CommonProxyErrors
    saved_proxy_server = sys.modules.get(proxy_server.__name__)
    saved_environment = {
        name: value for name, value in os.environ.items() if name.startswith("CYBERARK_")
    }
    saved_client = litellm.secret_manager_client
    saved_system = litellm._key_management_system  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    for name in saved_environment:
        del os.environ[name]
    for name, value in env.items():
        os.environ[name] = as_text(value)
    sys.modules[proxy_server.__name__] = proxy_server

    def sync_handler(**options: Json) -> SimpleNamespace:
        client = Client(transport=transport, verify=ssl_verify(options), cert=None, blocking=True)
        return SimpleNamespace(client=client)

    def async_handler(**options: Json) -> SimpleNamespace:
        client = Client(transport=transport, verify=ssl_verify(options), cert=None, blocking=False)
        return SimpleNamespace(
            client=SimpleNamespace(post=client.apost, get=client.aget),
            post=client.apost,
            get=client.aget,
        )

    def certificate_client(*, cert: tuple[str, str], verify: bool) -> Client:
        return Client(transport=transport, verify=verify, cert=cert, blocking=True)

    def clocked_cache(*, default_ttl: int) -> InMemoryCache:
        return InMemoryCache(default_ttl=default_ttl, clock=clock.read)

    def no_native(*_arguments: object) -> None:
        return None

    timers = SimpleNamespace(Lock=asyncio.Lock, sleep=clock.sleep)
    try:
        with ExitStack() as stack:
            stack.enter_context(patch.object(cyberark, "_get_httpx_client", sync_handler))
            stack.enter_context(patch.object(cyberark, "get_async_httpx_client", async_handler))
            stack.enter_context(patch.object(httpx, "Client", certificate_client))
            stack.enter_context(patch.object(cyberark, "InMemoryCache", clocked_cache))
            stack.enter_context(patch.object(cyberark, "asyncio", timers))
            stack.enter_context(patch.object(cyberark, "resolve_native_provider_reader", no_native))
            stack.enter_context(patch.object(cyberark, "resolve_native_provider_writer", no_native))
            yield
    finally:
        if saved_proxy_server is None:
            del sys.modules[proxy_server.__name__]
        else:
            sys.modules[proxy_server.__name__] = saved_proxy_server
        for name in [name for name in os.environ if name.startswith("CYBERARK_")]:
            del os.environ[name]
        os.environ.update(saved_environment)
        litellm.secret_manager_client = saved_client
        litellm._key_management_system = saved_system  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]


def operation(result: dict[str, str]) -> Json:
    status = result["status"]
    return {"status": status, "message": None if status == "error" else result["message"]}


async def run_step(manager: Manager, step: Object, clock: Clock) -> Json:
    match as_text(step["op"]):
        case "read":
            return {"value": await manager.async_read_secret(as_text(step["name"]))}
        case "sync_read":
            return {"value": manager.sync_read_secret(as_text(step["name"]))}
        case "write":
            written = await manager.async_write_secret(
                as_text(step["name"]), as_text(step["value"])
            )
            return operation(written)
        case "delete":
            return operation(await manager.async_delete_secret(as_text(step["name"])))
        case "rotate":
            return await rotate(manager, step)
        case "advance":
            clock.now += as_int(step["ms"]) / 1000
            return None
        case other:
            raise ValueError(f"unknown b4418 step: {other}")


async def rotate(manager: Manager, step: Object) -> Json:
    try:
        rotated = await manager.async_rotate_secret(
            as_text(step["current"]), as_text(step["new"]), as_text(step["value"])
        )
    except ValueError as error:
        missing_current = str(error).startswith("Current secret")
        return {
            "error": "current_secret_not_found" if missing_current else "new_secret_not_verified"
        }
    return operation(rotated)


async def execute(input_value: Object, clock: Clock, transport: Transport) -> Json:
    capture = LogCapture()
    saved_level = verbose_logger.level
    saved_handlers = verbose_logger.handlers[:]
    verbose_logger.handlers = [capture]
    verbose_logger.setLevel(logging.DEBUG)
    steps: list[Json] = []
    try:
        try:
            manager = cast(Manager, cyberark.CyberArkSecretManager())
        except ValueError:
            return {"init": {"ok": False, "logs": capture.take()}, "steps": steps}
        init: Object = {"ok": True, "logs": capture.take()}
        for step in as_list(input_value["steps"]):
            result = await run_step(manager, as_object(step), clock)
            steps.append({"result": result, "logs": capture.take(), "requests": transport.requests})
            transport.requests = []
        return {"init": init, "steps": steps}
    finally:
        verbose_logger.handlers = saved_handlers
        verbose_logger.setLevel(saved_level)


def run(input_value: Object) -> Json:
    if set(input_value) != FIELDS:
        raise ValueError(f"input fields must be {sorted(FIELDS)}")
    clock = Clock()
    transport = Transport(as_list(input_value["routes"]), clock)
    env = as_object(input_value["env"])
    with patched_world(env, transport, clock):
        return asyncio.run(execute(input_value, clock, transport))
