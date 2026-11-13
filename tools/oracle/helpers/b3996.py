import asyncio
import importlib
import json
import os
from collections.abc import Awaitable, Callable
from datetime import timedelta
from typing import Protocol, TypeAlias, cast

from litellm.integrations.gcs_bucket import gcs_bucket_mock_client
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.llms.vertex_ai.vertex_llm_base import VertexBase

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Fields: TypeAlias = dict[str, Json]
Handler: TypeAlias = Callable[..., Awaitable["ObservedResponse"]]

ENABLE_VARIABLE = "GCS_MOCK"
LATENCY_VARIABLE = "GCS_MOCK_LATENCY_MS"
DEFAULT_INSTALLS = 1
ORIGINAL_TOKEN = "original-token"  # noqa: S105 - a visible sentinel, not a credential
ORIGINAL_PROJECT = "original-project"
ORIGINAL_URL = "https://original.example.com"


class InputError(ValueError):
    pass


class GcsMockModule(Protocol):
    def should_use_gcs_mock(self) -> bool: ...

    def create_mock_gcs_client(self) -> None: ...

    def mock_vertex_auth_methods(self) -> None: ...


class ObservedResponse(Protocol):
    status_code: int
    text: str
    url: object
    elapsed: timedelta


class OriginalResponse:
    def __init__(self, status: int, body: str) -> None:
        self.status_code = status
        self.text = body
        self.content = body.encode()


class Recorder:
    def __init__(self) -> None:
        self.entries: list[Fields] = []


class OriginalGet:
    def __init__(self, recorder: Recorder, response: OriginalResponse) -> None:
        self.recorder = recorder
        self.response = response

    async def __call__(
        self,
        _handler: AsyncHTTPHandler,
        url: str,
        params: Fields | None = None,
        headers: Fields | None = None,
        follow_redirects: bool | None = None,  # noqa: FBT001 - LiteLLM's keyword name
    ) -> OriginalResponse:
        self.recorder.entries.append(
            {
                "method": "get",
                "url": url,
                "params": {} if params is None else params,
                "headers": {} if headers is None else headers,
                "follow_redirects": follow_redirects,
            }
        )
        return self.response


class OriginalWrite:
    def __init__(self, recorder: Recorder, response: OriginalResponse, method: str) -> None:
        self.recorder = recorder
        self.response = response
        self.method = method

    async def __call__(  # noqa: PLR0913, PLR0917 - LiteLLM's httpx handler signature
        self,
        _handler: AsyncHTTPHandler,
        url: str,
        data: Json = None,
        json: Json = None,
        params: Fields | None = None,
        headers: Fields | None = None,
        timeout: float | None = None,  # noqa: ASYNC109 - LiteLLM's keyword name
        stream: bool = False,  # noqa: FBT001, FBT002 - LiteLLM's keyword name
        logging_obj: object = None,  # noqa: ARG002 - LiteLLM's keyword name
        files: object = None,  # noqa: ARG002 - LiteLLM's keyword name
        content: str | None = None,
    ) -> OriginalResponse:
        self.recorder.entries.append(
            {
                "method": self.method,
                "url": url,
                "data": data,
                "json": json,
                "params": {} if params is None else params,
                "headers": {} if headers is None else headers,
                "timeout_ms": None if timeout is None else round(timeout * 1000),
                "stream": stream,
                "content": content,
            }
        )
        return self.response


def object_value(value: Json, name: str) -> Fields:
    if not isinstance(value, dict):
        raise InputError(f"{name} must be an object")
    return value


def list_value(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise InputError(f"{name} must be a list")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise InputError(f"{name} must be a string")
    return value


def integer_value(value: Json, name: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise InputError(f"{name} must be an integer")
    return value


def boolean_value(value: Json, name: str) -> bool:
    if not isinstance(value, bool):
        raise InputError(f"{name} must be a boolean")
    return value


def apply_environment(environment: Fields) -> None:
    for name in (ENABLE_VARIABLE, LATENCY_VARIABLE):
        value = environment.get(name)
        if value is None:
            os.environ.pop(name, None)
            continue
        os.environ[name] = string_value(value, name)


def load_module() -> GcsMockModule:
    return cast(GcsMockModule, importlib.reload(gcs_bucket_mock_client))


def normalize(value: Json) -> Json:
    if isinstance(value, float) and value.is_integer():
        return int(value)
    if isinstance(value, list):
        return [normalize(item) for item in value]
    if isinstance(value, dict):
        return {key: normalize(item) for key, item in value.items()}
    return value


def observed_body(text: str) -> Json:
    try:
        parsed = cast(Json, json.loads(text))
    except json.JSONDecodeError:
        return text
    return normalize(parsed)


def response_call(
    name: str, handler: AsyncHTTPHandler, **kwargs: Json
) -> Awaitable[ObservedResponse]:
    call = cast(Handler, getattr(AsyncHTTPHandler, name))
    return call(handler, **kwargs)


async def run_request(handler: AsyncHTTPHandler, recorder: Recorder, request: Json) -> Json:
    fields = object_value(request, "request")
    method = string_value(fields["method"], "method")
    before = len(recorder.entries)
    if method == "get":
        response = await response_call(
            "get",
            handler,
            url=string_value(fields["url"], "url"),
            params=object_value(fields.get("params", {}), "params"),
            headers=object_value(fields.get("headers", {}), "headers"),
            follow_redirects=None
            if fields.get("follow_redirects") is None
            else boolean_value(fields["follow_redirects"], "follow_redirects"),
        )
    elif method in ("post", "delete"):
        timeout_ms = fields.get("timeout_ms")
        response = await response_call(
            method,
            handler,
            url=string_value(fields["url"], "url"),
            data=fields.get("data"),
            json=fields.get("json"),
            params=object_value(fields.get("params", {}), "params"),
            headers=object_value(fields.get("headers", {}), "headers"),
            timeout=None if timeout_ms is None else integer_value(timeout_ms, "timeout_ms") / 1000,
            stream=boolean_value(fields.get("stream", False), "stream"),
            content=fields.get("content"),
        )
    else:
        raise InputError(f"unknown method {method}")
    mocked = len(recorder.entries) == before
    observed: Fields = {
        "mocked": mocked,
        "status": response.status_code,
        "body": observed_body(response.text),
    }
    if mocked:
        observed["url"] = str(response.url)
        observed["elapsed_ms"] = round(response.elapsed.total_seconds() * 1000)
    else:
        observed["received"] = recorder.entries[-1]
    return observed


async def run_requests(module: GcsMockModule, input_value: Fields) -> list[Json]:
    enabled = module.should_use_gcs_mock()
    original = input_value.get("original")
    recorder = Recorder()
    if original is None:
        setattr(AsyncHTTPHandler, "get", None)  # noqa: B010 - a missing original is the case's setup
        setattr(AsyncHTTPHandler, "post", None)  # noqa: B010
        setattr(AsyncHTTPHandler, "delete", None)  # noqa: B010
    else:
        fields = object_value(original, "original")
        response = OriginalResponse(
            integer_value(fields["status"], "status"), string_value(fields["body"], "body")
        )
        setattr(AsyncHTTPHandler, "get", OriginalGet(recorder, response))  # noqa: B010
        setattr(AsyncHTTPHandler, "post", OriginalWrite(recorder, response, "post"))  # noqa: B010
        setattr(AsyncHTTPHandler, "delete", OriginalWrite(recorder, response, "delete"))  # noqa: B010
    if enabled:
        module.create_mock_gcs_client()
    handler = AsyncHTTPHandler()
    return [
        await run_request(handler, recorder, request)
        for request in list_value(input_value["requests"], "requests")
    ]


async def original_async_token(
    _self: VertexBase,
    credentials: str | None,  # noqa: ARG001 - LiteLLM's keyword name
    project_id: str | None,  # noqa: ARG001 - LiteLLM's keyword name
    custom_llm_provider: str,  # noqa: ARG001 - LiteLLM's keyword name
) -> tuple[str, str]:
    return (ORIGINAL_TOKEN, ORIGINAL_PROJECT)


def original_sync_token(
    _self: VertexBase,
    credentials: str | None,  # noqa: ARG001 - LiteLLM's keyword name
    project_id: str | None,  # noqa: ARG001 - LiteLLM's keyword name
    custom_llm_provider: str,  # noqa: ARG001 - LiteLLM's keyword name
) -> tuple[str, str]:
    return (ORIGINAL_TOKEN, ORIGINAL_PROJECT)


def original_token_and_url(  # noqa: PLR0913, PLR0917 - LiteLLM's keyword names
    _self: VertexBase,
    model: str,  # noqa: ARG001
    auth_header: str | None,  # noqa: ARG001
    vertex_credentials: str | None,  # noqa: ARG001
    vertex_project: str | None,  # noqa: ARG001
    vertex_location: str | None,  # noqa: ARG001
    gemini_api_key: str | None,  # noqa: ARG001
    stream: bool | None,  # noqa: ARG001, FBT001 - LiteLLM's keyword name
    custom_llm_provider: str,  # noqa: ARG001
    api_base: str | None,  # noqa: ARG001
) -> tuple[str, str]:
    return (ORIGINAL_TOKEN, ORIGINAL_URL)


def install_vertex_originals() -> None:
    for name in (
        "_original_ensure_access_token_async",
        "_original_ensure_access_token",
        "_original_get_token_and_url",
    ):
        if hasattr(VertexBase, name):
            delattr(VertexBase, name)
    setattr(VertexBase, "_ensure_access_token_async", original_async_token)  # noqa: B010
    setattr(VertexBase, "_ensure_access_token", original_sync_token)  # noqa: B010
    setattr(VertexBase, "_get_token_and_url", original_token_and_url)  # noqa: B010


async def run_auth(module: GcsMockModule, input_value: Fields) -> Fields:
    installs = integer_value(input_value.get("installs", DEFAULT_INSTALLS), "installs")
    install_vertex_originals()
    outcomes: list[Json] = []
    for _ in range(installs):
        installed = not hasattr(VertexBase, "_original_ensure_access_token_async")
        module.mock_vertex_auth_methods()
        outcomes.append("installed" if installed else "already_installed")
    instance = object.__new__(VertexBase)
    asynchronous = await VertexBase._ensure_access_token_async(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        instance, credentials=None, project_id=None, custom_llm_provider="vertex_ai"
    )
    synchronous = VertexBase._ensure_access_token(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        instance, credentials=None, project_id=None, custom_llm_provider="vertex_ai"
    )
    token_and_url = VertexBase._get_token_and_url(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        instance,
        model="gcs-bucket",
        auth_header=None,
        vertex_credentials=None,
        vertex_project=None,
        vertex_location=None,
        gemini_api_key=None,
        stream=None,
        custom_llm_provider="vertex_ai",
        api_base=None,
    )
    return {
        "installs": outcomes,
        "async_token": {"token": asynchronous[0], "project_id": asynchronous[1]},
        "sync_token": {"token": synchronous[0], "project_id": synchronous[1]},
        "token_and_url": {"token": token_and_url[0], "url": token_and_url[1]},
    }


async def execute(input_value: Fields) -> Fields:
    environment = object_value(input_value.get("environment", {}), "environment")
    apply_environment(environment)
    module = load_module()
    result: Fields = {"enabled": module.should_use_gcs_mock()}
    if "requests" in input_value:
        result["requests"] = await run_requests(module, input_value)
    auth = input_value.get("auth")
    if auth is not None:
        result["auth"] = await run_auth(module, object_value(auth, "auth"))
    return result


def run(input_value: Fields) -> Json:
    return asyncio.run(execute(input_value))
