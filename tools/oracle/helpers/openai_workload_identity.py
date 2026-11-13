import os
import sys
import tempfile
import threading
from collections.abc import Callable, Generator
from concurrent.futures import ThreadPoolExecutor
from contextlib import ExitStack, contextmanager
from pathlib import Path
from types import ModuleType
from typing import Self, TypeAlias
from unittest import mock

import httpx
import litellm
import openai.auth
from litellm.llms.openai import workload_identity
from litellm.llms.openai.common_utils import OpenAIError as LiteLLMOpenAIError
from litellm.llms.openai.workload_identity import (
    OpenAIWorkloadIdentityConfig,
    get_workload_identity_bearer_token,
    resolve_openai_workload_identity_config,
)
from openai import OAuthError, OpenAIError
from openai._exceptions import SubjectTokenProviderError  # pyright: ignore[reportPrivateUsage]
from openai.auth import _workload  # pyright: ignore[reportPrivateUsage]

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]

SDK_NAMES = {
    "available": (),
    "no_auth_class": ("WorkloadIdentityAuth",),
    "no_k8s_provider": ("k8s_service_account_token_provider",),
    "none": ("WorkloadIdentityAuth", "k8s_service_account_token_provider"),
}


def obj(value: Json) -> Object:
    if not isinstance(value, dict):
        raise TypeError("expected an object")
    return value


def items(value: Json) -> list[Json]:
    if not isinstance(value, list):
        raise TypeError("expected a list")
    return value


def text(fields: Object, name: str) -> str:
    value = fields.get(name)
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def optional_text(fields: Object, name: str) -> str | None:
    value = fields.get(name)
    if value is not None and not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def whole(fields: Object, name: str) -> int:
    value = fields.get(name)
    if isinstance(value, bool) or not isinstance(value, int):
        raise TypeError(f"{name} must be an integer")
    return value


def environment(fields: Object) -> dict[str, str]:
    raw = obj(fields.get("env", {}))
    return {name: text(raw, name) for name in raw}


class Transport:
    def __init__(self, replies: list[Json]) -> None:
        self.replies = [obj(reply) for reply in replies]
        self.requests: list[Json] = []
        self.guard = threading.Lock()

    def post(self, url: str, *, json: Object, timeout: float) -> httpx.Response:
        with self.guard:
            self.requests.append(
                {
                    "method": "POST",
                    "url": url,
                    "json": json,
                    "timeout_ms": round(timeout * 1000),
                }
            )
            reply: Object = self.replies.pop(0) if self.replies else {"status": 500, "body": ""}
        if reply.get("failure") is True:
            raise httpx.ConnectError("connection refused")
        return httpx.Response(
            whole(reply, "status"),
            content=text(reply, "body").encode(),
            request=httpx.Request("POST", url),
        )

    def __enter__(self) -> Self:
        return self

    def __exit__(self, *_exc: object) -> None:
        return None

    def drain(self) -> list[Json]:
        with self.guard:
            taken = self.requests
            self.requests = []
        return taken


class Clock:
    def __init__(self) -> None:
        self.milliseconds = 0

    def monotonic(self) -> float:
        return self.milliseconds / 1000


def sdk_module(variant: str) -> ModuleType:
    module = ModuleType("openai.auth")
    removed = SDK_NAMES[variant]
    for name in ("WorkloadIdentityAuth", "k8s_service_account_token_provider"):
        if name not in removed:
            setattr(module, name, getattr(openai.auth, name))
    return module


@contextmanager
def patched_world(
    transport: Transport, clock: Clock, variant: str
) -> Generator[None]:
    workload_identity._workload_identity_auth.cache_clear()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    with ExitStack() as stack:
        stack.enter_context(mock.patch.object(httpx, "Client", return_value=transport))
        stack.enter_context(mock.patch.object(_workload, "time", clock))
        stack.enter_context(mock.patch.dict(sys.modules, {"openai.auth": sdk_module(variant)}))
        try:
            yield
        finally:
            workload_identity._workload_identity_auth.cache_clear()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]


def probe(fields: Object) -> Json:
    with mock.patch.dict(os.environ, environment(fields), clear=True):
        litellm.api_base = optional_text(fields, "litellm_api_base")
        found = resolve_openai_workload_identity_config(
            optional_text(fields, "api_key"), optional_text(fields, "api_base")
        )
    if found is None:
        return {"config": None}
    return {
        "config": {
            "identity_provider_id": found.identity_provider_id,
            "service_account_id": found.service_account_id,
            "token_file": found.token_file,
        }
    }


def resolve(fields: Object) -> Json:
    saved_base = litellm.api_base
    try:
        return {"results": [probe(obj(entry)) for entry in items(fields["probes"])]}
    finally:
        litellm.api_base = saved_base


def identity_config(directory: str, identity: Object) -> OpenAIWorkloadIdentityConfig:
    name = text(identity, "name")
    return OpenAIWorkloadIdentityConfig(
        identity_provider_id=optional_text(identity, "provider") or f"idp-{name}",
        service_account_id=optional_text(identity, "account") or f"sa-{name}",
        token_file=str(Path(directory) / name),
    )


def failure(error: Exception, directory: str) -> Json:
    if isinstance(error, OAuthError):
        return {
            "kind": "oauth",
            "status": error.status_code,
            "error": error.error,
            "message": error.message,
        }
    if isinstance(error, SubjectTokenProviderError):
        return {"kind": "subject_token", "message": str(error).replace(directory, "<dir>")}
    if isinstance(error, OpenAIError):
        return {"kind": "openai", "message": str(error)}
    if isinstance(error, LiteLLMOpenAIError):
        return {"kind": "sdk_unavailable", "status": error.status_code, "message": error.message}
    if isinstance(error, RuntimeError):
        return {"kind": "runtime", "message": str(error)}
    if isinstance(error, httpx.TransportError):
        return {"kind": "transport"}
    raise error


STEP_ERRORS = (OAuthError, OpenAIError, LiteLLMOpenAIError, RuntimeError, httpx.TransportError)


def token_outcome(config: OpenAIWorkloadIdentityConfig, directory: str) -> Json:
    try:
        return {"token": get_workload_identity_bearer_token(config)}
    except STEP_ERRORS as error:
        return {"error": failure(error, directory)}


def token_step(step: Object, directory: str) -> Json:
    return token_outcome(identity_config(directory, obj(step["identity"])), directory)


def call_after(
    start: threading.Barrier, config: OpenAIWorkloadIdentityConfig, directory: str
) -> Json:
    start.wait()
    return token_outcome(config, directory)


def concurrent_step(step: Object, directory: str) -> Json:
    config = identity_config(directory, obj(step["identity"]))
    callers = whole(step, "callers")
    start = threading.Barrier(callers)
    with ThreadPoolExecutor(max_workers=callers) as pool:
        running = [pool.submit(call_after, start, config, directory) for _ in range(callers)]
        return {"outcomes": [future.result() for future in running]}


def auth_step(step: Object, directory: str, auths: dict[str, object]) -> Json:
    config = identity_config(directory, obj(step["identity"]))
    try:
        found = workload_identity._workload_identity_auth(config)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    except STEP_ERRORS as error:
        return {"error": failure(error, directory)}
    same_as: list[Json] = [label for label, earlier in auths.items() if earlier is found]
    auths[text(step, "label")] = found
    return {"same_as": same_as}


def run_step(
    step: Object, directory: str, clock: Clock, auths: dict[str, object]
) -> Json:
    operation = text(step, "op")
    match operation:
        case "advance":
            clock.milliseconds += whole(step, "ms")
            return None
        case "token":
            return token_step(step, directory)
        case "concurrent":
            return concurrent_step(step, directory)
        case "auth":
            return auth_step(step, directory, auths)
        case _:
            raise ValueError(f"unknown openai_workload_identity step: {operation}")


def flow(fields: Object) -> Json:
    transport = Transport(items(fields["replies"]))
    clock = Clock()
    variant = optional_text(fields, "sdk") or "available"
    auths: dict[str, object] = {}
    steps: list[Json] = []
    with tempfile.TemporaryDirectory() as directory:
        for name, contents in obj(fields.get("files", {})).items():
            if not isinstance(contents, str):
                raise TypeError("token file contents must be text")
            (Path(directory) / name).write_text(contents)
        with patched_world(transport, clock, variant):
            for entry in items(fields["steps"]):
                result = run_step(obj(entry), directory, clock, auths)
                steps.append({"result": result, "requests": transport.drain()})
    return {"steps": steps}


HANDLERS: dict[str, Callable[[Object], Json]] = {"resolve": resolve, "flow": flow}


def run(argument: Json) -> Json:
    fields = obj(argument)
    if len(fields) != 1:
        raise ValueError("expected one openai_workload_identity variant")
    name, value = next(iter(fields.items()))
    handler = HANDLERS.get(name)
    if handler is None:
        raise ValueError(f"unknown openai_workload_identity case: {name}")
    return handler(obj(value))
