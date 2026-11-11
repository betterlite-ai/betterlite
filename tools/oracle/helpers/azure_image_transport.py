import asyncio
import importlib
import json as jsonlib
import os
from collections.abc import Mapping
from typing import TYPE_CHECKING, Protocol, TypeAlias, TypedDict, cast
from unittest import mock

import httpx
import litellm.constants
import litellm.llms.azure.azure as azure_module
from litellm.llms.azure.azure import AzureChatCompletion
from litellm.llms.azure.common_utils import AzureOpenAIError

if TYPE_CHECKING:
    from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler, HTTPHandler

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

TIMEOUT_ENV = "AZURE_OPERATION_POLLING_TIMEOUT"
DEFAULT_POLLING_TIMEOUT: int = litellm.constants.AZURE_OPERATION_POLLING_TIMEOUT


class Reply(TypedDict, total=False):
    status: int
    headers: dict[str, str]
    body: str
    delay: float


class Step(TypedDict, total=False):
    environment: dict[str, str]
    api_base: str
    api_version: str
    deployment_name: str | None
    headers: dict[str, str]
    data: dict[str, Json]
    is_async: bool
    supplied_client: bool
    timeout: float | None
    replies: list[Reply]


class Input(TypedDict):
    steps: list[Step]


class ResponseLike(Protocol):
    @property
    def status_code(self) -> int: ...

    @property
    def headers(self) -> Mapping[str, str]: ...

    @property
    def request(self) -> httpx.Request: ...

    def json(self) -> Json: ...


class Clock:
    def __init__(self) -> None:
        self.elapsed = 0.0
        self.sleeps: list[int] = []

    def time(self) -> float:
        return self.elapsed

    async def async_sleep(self, delay: float) -> None:
        self.sleeps.append(int(delay))
        if delay > 0:
            self.elapsed += delay

    def sync_sleep(self, delay: float) -> None:
        if delay < 0:
            raise ValueError("sleep length must be non-negative")
        self.sleeps.append(int(delay))
        self.elapsed += delay


class FixtureResponse:
    def __init__(self, reply: Reply, method: str, url: str) -> None:
        self.status_code = reply.get("status", 200)
        self.headers = dict(reply.get("headers", {}))
        self.request = httpx.Request(method, url)
        self._text = reply.get("body", "")

    @property
    def text(self) -> str:
        return self._text

    @property
    def content(self) -> bytes:
        return self._text.encode()

    def json(self) -> Json:
        return cast("Json", jsonlib.loads(self._text))

    def read(self) -> bytes:
        return self.content

    async def aread(self) -> bytes:
        return self.content

    def raise_for_status(self) -> None:
        return None


def request_body(data: str | None, json_body: Json) -> Json:
    if data is None:
        return json_body
    return cast("Json", jsonlib.loads(data))


def timeout_argument(kwargs: dict[str, object]) -> Json:
    return cast("Json", kwargs.get("timeout"))


class State:
    def __init__(self, step: Step, clock: Clock) -> None:
        self.replies = list(step.get("replies", []))
        self.clock = clock
        self.calls: list[Json] = []
        self.constructed: list[Json] = []
        self.last_headers: dict[str, str] = {}

    def answer(
        self,
        method: str,
        url: str,
        headers: Mapping[str, str] | None,
        body: Json,
        timeout: Json,
    ) -> FixtureResponse:
        call: Json = {
            "method": method,
            "url": url,
            "headers": dict(headers or {}),
            "body": body,
            "timeout": timeout,
        }
        self.calls.append(call)
        reply = self.replies.pop(0)
        self.last_headers = dict(reply.get("headers", {}))
        delay = reply.get("delay", 0)
        if delay > 0:
            self.clock.elapsed += delay
        return FixtureResponse(reply, method, url)


class SyncHandler:
    def __init__(self, state: State) -> None:
        self.state = state

    def post(
        self,
        url: str,
        data: str | None = None,
        json: Json = None,
        headers: Mapping[str, str] | None = None,
        **kwargs: object,
    ) -> FixtureResponse:
        return self.state.answer(
            "POST", url, headers, request_body(data, json), timeout_argument(kwargs)
        )

    def get(
        self,
        url: str,
        headers: Mapping[str, str] | None = None,
        **kwargs: object,
    ) -> FixtureResponse:
        return self.state.answer("GET", url, headers, None, timeout_argument(kwargs))


class AsyncHandler:
    def __init__(self, state: State) -> None:
        self.state = state

    async def post(
        self,
        url: str,
        data: str | None = None,
        json: Json = None,
        headers: Mapping[str, str] | None = None,
        **kwargs: object,
    ) -> FixtureResponse:
        return self.state.answer(
            "POST", url, headers, request_body(data, json), timeout_argument(kwargs)
        )

    async def get(
        self,
        url: str,
        headers: Mapping[str, str] | None = None,
        **kwargs: object,
    ) -> FixtureResponse:
        return self.state.answer("GET", url, headers, None, timeout_argument(kwargs))


def timeout_phases(value: object) -> Json:
    if isinstance(value, httpx.Timeout):
        return cast(
            "Json",
            {
                "connect": value.connect,
                "read": value.read,
                "write": value.write,
                "pool": value.pool,
            },
        )
    return None


class SyncConstructor:
    def __init__(self, state: State) -> None:
        self.state = state

    def __call__(self, **params: object) -> SyncHandler:
        self.state.constructed.append(timeout_phases(params.get("timeout")))
        return SyncHandler(self.state)


class AsyncFactory:
    def __init__(self, state: State) -> None:
        self.state = state

    def __call__(
        self,
        params: dict[str, object] | None = None,
        **_ignored: object,
    ) -> AsyncHandler:
        timeout = params.get("timeout") if params is not None else None
        self.state.constructed.append(timeout_phases(timeout))
        return AsyncHandler(self.state)


def polling_timeout(environment: dict[str, str]) -> int:
    if TIMEOUT_ENV not in environment:
        return DEFAULT_POLLING_TIMEOUT
    importlib.reload(litellm.constants)
    return litellm.constants.AZURE_OPERATION_POLLING_TIMEOUT


def common(state: State, data: dict[str, Json]) -> dict[str, Json]:
    calls = cast("Json", state.calls)
    constructed = cast("Json", state.constructed)
    sleeps = cast("Json", state.clock.sleeps)
    return {
        "data": cast("Json", data),
        "calls": calls,
        "sleeps": sleeps,
        "constructed": constructed,
    }


def response_headers(response: ResponseLike, state: State) -> dict[str, str]:
    allowed = {name.lower() for name in state.last_headers}
    return {
        name: value
        for name, value in response.headers.items()
        if name.lower() in allowed
    }


def response_value(response: ResponseLike, state: State, data: dict[str, Json]) -> Json:
    value: dict[str, Json] = {
        "kind": "response",
        "status": response.status_code,
        "headers": cast("Json", response_headers(response, state)),
        "body": response.json(),
        "request": {
            "method": response.request.method,
            "url": str(response.request.url),
        },
        "message_kind": None,
        **common(state, data),
    }
    return value


def failure_kind(body: Json) -> str | None:
    if not isinstance(body, dict):
        return None
    error_body: Json = body.get("error", body)
    if isinstance(error_body, dict):
        return "error_message" if "message" in error_body else "image_generation_failed"
    return "operation_json"


def azure_error_value(error: AzureOpenAIError, state: State, data: dict[str, Json]) -> Json:
    raw_body = error.body  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
    body = cast("Json", raw_body) if isinstance(raw_body, dict) else None
    value: dict[str, Json] = {
        "kind": "error",
        "status": error.status_code,
        "headers": None,
        "body": body,
        "request": None,
        "message_kind": failure_kind(body),
        **common(state, data),
    }
    return value


def generic_error_value(state: State, data: dict[str, Json]) -> Json:
    value: dict[str, Json] = {
        "kind": "error",
        "status": None,
        "headers": None,
        "body": None,
        "request": None,
        "message_kind": None,
        **common(state, data),
    }
    return value


def invoke(
    step: Step,
    client: SyncHandler | AsyncHandler | None,
    data: dict[str, Json],
    headers: dict[str, str],
    timeout: float | None,
) -> ResponseLike:
    azure = AzureChatCompletion()
    deployment_name = step.get("deployment_name")
    api_base = step.get("api_base", "")
    api_version = step.get("api_version", "")
    if step.get("is_async", False):
        return cast(
            "ResponseLike",
            asyncio.run(
                azure.make_async_azure_httpx_request(  # pyright: ignore[reportUnknownMemberType]
                    client=cast("AsyncHTTPHandler | None", client),
                    timeout=timeout,
                    api_base=api_base,
                    api_version=api_version,
                    api_key="",
                    data=cast("dict[str, object]", data),
                    headers=headers,
                    deployment_name=deployment_name,
                )
            ),
        )
    return cast(
        "ResponseLike",
        azure.make_sync_azure_httpx_request(  # pyright: ignore[reportUnknownMemberType]
            client=cast("HTTPHandler | None", client),
            timeout=timeout,
            api_base=api_base,
            api_version=api_version,
            api_key="",
            data=cast("dict[str, object]", data),
            headers=headers,
            deployment_name=deployment_name,
        ),
    )


def run_step(step: Step) -> Json:
    environment = dict(step.get("environment", {}))
    clock = Clock()
    state = State(step, clock)
    data = dict(step.get("data", {}))
    headers = dict(step.get("headers", {}))
    timeout = step.get("timeout")
    client: SyncHandler | AsyncHandler | None = None
    if step.get("supplied_client", False):
        client = AsyncHandler(state) if step.get("is_async", False) else SyncHandler(state)
    with (
        mock.patch.dict(os.environ, environment, clear=True),
        mock.patch.object(azure_module, "get_async_httpx_client", AsyncFactory(state)),
        mock.patch.object(azure_module, "HTTPHandler", SyncConstructor(state)),
        mock.patch("time.time", clock.time),
        mock.patch("time.sleep", clock.sync_sleep),
        mock.patch.object(asyncio, "sleep", clock.async_sleep),
    ):
        azure_module.AZURE_OPERATION_POLLING_TIMEOUT = polling_timeout(  # pyright: ignore[reportPrivateImportUsage]
            environment
        )
        try:
            response = invoke(step, client, data, headers, timeout)
        except AzureOpenAIError as error:
            return azure_error_value(error, state, data)
        except Exception:  # noqa: BLE001 - the case compares which failures the source raises
            return generic_error_value(state, data)
        return response_value(response, state, data)


def run(input_value: dict[str, Json]) -> Json:
    steps = cast("list[Step]", input_value.get("steps") or [])
    return cast("Json", [run_step(step) for step in steps])
