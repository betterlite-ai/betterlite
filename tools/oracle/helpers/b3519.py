import asyncio
import os
from collections.abc import Coroutine
from typing import Final, Literal, Protocol, Required, TypedDict

import httpx
from litellm.constants import OPEN_SANDBOX_DEFAULT_LANGUAGE
from litellm.llms.base_llm.sandbox.transformation import (
    CodeExecutionResult,
    ContainerHandle,
)
from litellm.sandbox import sandbox_tools
from litellm.types.utils import SandboxProviders
from litellm.utils import ProviderConfigManager
from pydantic import JsonValue, TypeAdapter

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class Reply(TypedDict, total=False):
    status: int
    body: str


class Input(TypedDict, total=False):
    operation: Required[Literal["registry", "create", "create_and_run", "delete"]]
    provider: str
    tools: list[dict[str, JsonValue]]
    names: list[str]
    environment: dict[str, str]
    template: str | None
    timeout: int | None
    allow_internet_access: bool | None
    api_key: str | None
    api_base: str | None
    metadata: dict[str, JsonValue] | None
    env_vars: dict[str, JsonValue] | None
    resource_limits: dict[str, JsonValue] | None
    resource_requests: dict[str, JsonValue] | None
    entrypoint: list[JsonValue] | None
    network_policy: dict[str, JsonValue] | None
    secure_access: bool | None
    use_server_proxy: bool | None
    ready_timeout: float | None
    poll_interval: float | None
    code: str
    language: str | None
    container: str
    replies: list[Reply]


class _Response(httpx.Response):
    def __init__(self, status: int, body: str) -> None:
        super().__init__(status, content=body.encode())


class _Handler:
    def __init__(self, replies: list[Reply]) -> None:
        self._replies = replies
        self.requests: list[JsonValue] = []

    def _record(
        self, method: str, url: str, headers: dict[str, str], body: JsonValue
    ) -> None:
        recorded: dict[str, JsonValue] = {
            "method": method,
            "url": url,
            "headers": {name.lower(): value for name, value in headers.items()},
            "json": body,
        }
        self.requests.append(recorded)

    def _next(self) -> _Response:
        if not self._replies:
            return _Response(200, "")
        reply = self._replies.pop(0) if len(self._replies) > 1 else self._replies[0]
        status = reply.get("status")
        body = reply.get("body")
        return _Response(
            200 if status is None else status,
            "" if body is None else body,
        )

    async def post(
        self,
        url: str,
        headers: dict[str, str],
        json: dict[str, JsonValue],
        **options: JsonValue,
    ) -> _Response:
        del options
        self._record("POST", url, headers, json)
        return self._next()

    async def get(
        self, url: str, headers: dict[str, str], params: dict[str, bool] | None = None
    ) -> _Response:
        target = url if params is None else str(httpx.URL(url, params=params))
        self._record("GET", target, headers, None)
        return self._next()

    async def delete(self, url: str, headers: dict[str, str]) -> _Response:
        self._record("DELETE", url, headers, None)
        response = self._next()
        if not httpx.codes.is_success(response.status_code):
            request = httpx.Request("DELETE", url)
            raise httpx.HTTPStatusError(
                "the API answered with an error status",
                request=request,
                response=httpx.Response(response.status_code, request=request),
            )
        return response


# the provider config's async entry points, typed as the coroutines asyncio.run takes
class SandboxConfig(Protocol):
    def acreate_sandbox(
        self,
        *,
        template: str | None = None,
        timeout: int | None = None,
        allow_internet_access: bool | None = None,
        api_key: str | None = None,
        **kwargs: JsonValue | _Handler,
    ) -> Coroutine[None, None, ContainerHandle]: ...

    def arun_code(
        self,
        *,
        container: ContainerHandle | str,
        code: str,
        api_key: str | None = None,
        **kwargs: JsonValue | _Handler,
    ) -> Coroutine[None, None, CodeExecutionResult]: ...

    def adelete_sandbox(
        self,
        *,
        container: ContainerHandle | str,
        api_key: str | None = None,
        **kwargs: JsonValue | _Handler,
    ) -> Coroutine[None, None, bool]: ...


class SandboxToolRegistry(Protocol):
    def register_sandbox_tools(self, tools: list[dict[str, JsonValue]]) -> None: ...

    def resolve_sandbox_tool(self, name: str) -> dict[str, JsonValue] | None: ...


def config_for(provider: str) -> SandboxConfig:
    config = ProviderConfigManager.get_provider_sandbox_config(SandboxProviders(provider))
    if config is None:
        raise ValueError(f"Code execution is not supported for provider: {provider}")
    return config


def handler_for(input_value: Input) -> _Handler:
    replies = input_value.get("replies")
    return _Handler([] if replies is None else replies)


def create_sandbox(
    config: SandboxConfig, input_value: Input, handler: _Handler
) -> ContainerHandle:
    return asyncio.run(
        config.acreate_sandbox(
            template=input_value.get("template"),
            timeout=input_value.get("timeout"),
            allow_internet_access=input_value.get("allow_internet_access"),
            api_key=input_value.get("api_key"),
            api_base=input_value.get("api_base"),
            metadata=input_value.get("metadata"),
            env_vars=input_value.get("env_vars"),
            resource_limits=input_value.get("resource_limits"),
            resource_requests=input_value.get("resource_requests"),
            entrypoint=input_value.get("entrypoint"),
            network_policy=input_value.get("network_policy"),
            secure_access=bool(input_value.get("secure_access")),
            use_server_proxy=bool(input_value.get("use_server_proxy")),
            ready_timeout=input_value.get("ready_timeout"),
            poll_interval=input_value.get("poll_interval"),
            client=handler,
        )
    )


def registry_answer(input_value: Input, registry: SandboxToolRegistry) -> JsonValue:
    tools = input_value.get("tools")
    names = input_value.get("names")
    if tools is None or names is None:
        raise ValueError("the registry operation needs tools and names")
    registry.register_sandbox_tools(tools)
    resolved: dict[str, JsonValue] = {}
    for name in names:
        resolved[name] = registry.resolve_sandbox_tool(name)
    answer: dict[str, JsonValue] = {"resolved": resolved}
    return JSON.validate_python(answer)


def create_answer(input_value: Input) -> JsonValue:
    provider = input_value.get("provider")
    if provider is None:
        raise ValueError("the create operation needs a provider")
    handler = handler_for(input_value)
    handle = create_sandbox(config_for(provider), input_value, handler)
    answer: dict[str, JsonValue] = {
        "requests": handler.requests,
        "handle": JSON.validate_json(handle.model_dump_json()),
    }
    return JSON.validate_python(answer)


def create_and_run_answer(input_value: Input) -> JsonValue:
    provider = input_value.get("provider")
    code = input_value.get("code")
    if provider is None or code is None:
        raise ValueError("the create_and_run operation needs a provider and code")
    handler = handler_for(input_value)
    config = config_for(provider)
    handle = create_sandbox(config, input_value, handler)
    language = input_value.get("language")
    result = asyncio.run(
        config.arun_code(
            container=handle,
            code=code,
            api_key=input_value.get("api_key"),
            api_base=input_value.get("api_base"),
            env_vars=input_value.get("env_vars"),
            language=OPEN_SANDBOX_DEFAULT_LANGUAGE if language is None else language,
            use_server_proxy=bool(input_value.get("use_server_proxy")),
            ready_timeout=input_value.get("ready_timeout"),
            poll_interval=input_value.get("poll_interval"),
            client=handler,
        )
    )
    answer: dict[str, JsonValue] = {
        "requests": handler.requests,
        "handle": JSON.validate_json(handle.model_dump_json()),
        "result": JSON.validate_json(result.model_dump_json()),
    }
    return JSON.validate_python(answer)


def delete_answer(input_value: Input) -> JsonValue:
    handler = handler_for(input_value)
    provider = input_value.get("provider")
    container = input_value.get("container")
    if provider is None or container is None:
        raise ValueError("the delete operation needs a provider and a container id")
    deleted = asyncio.run(
        config_for(provider).adelete_sandbox(
            container=container,
            api_key=input_value.get("api_key"),
            api_base=input_value.get("api_base"),
            client=handler,
        )
    )
    answer: dict[str, JsonValue] = {"requests": handler.requests, "deleted": deleted}
    return JSON.validate_python(answer)


def run(input_value: Input) -> JsonValue:
    environment = input_value.get("environment")
    if environment is not None:
        for name, value in environment.items():
            os.environ[name] = value
    operation = input_value["operation"]
    if operation == "registry":
        return registry_answer(input_value, sandbox_tools)
    if operation == "create":
        return create_answer(input_value)
    if operation == "create_and_run":
        return create_and_run_answer(input_value)
    return delete_answer(input_value)
