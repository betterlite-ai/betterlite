import asyncio
import os
from collections.abc import AsyncIterator
from typing import Final, Literal, Required, TypedDict

import httpx
from litellm.constants import OPEN_SANDBOX_DEFAULT_LANGUAGE
from litellm.llms.base_llm.sandbox.transformation import (
    BaseSandboxConfig,
    CodeExecutionResult,
    ContainerHandle,
)
from litellm.sandbox.sandbox_tools import register_sandbox_tools, resolve_sandbox_tool
from litellm.types.utils import SandboxProviders
from litellm.utils import ProviderConfigManager
from pydantic import JsonValue, TypeAdapter

JSON: Final = TypeAdapter(JsonValue)
DEFAULT_STATUS: Final = 200


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
    metadata: dict[str, str] | None
    env_vars: dict[str, str] | None
    resource_limits: dict[str, str] | None
    resource_requests: dict[str, str] | None
    entrypoint: list[str] | None
    network_policy: dict[str, JsonValue] | None
    secure_access: bool | None
    use_server_proxy: bool | None
    ready_timeout: float | None
    poll_interval: float | None
    code: str
    language: str | None
    container: str
    replies: list[Reply]


class _Response:
    def __init__(self, status: int, body: str) -> None:
        self.status_code = status
        self._body = body

    def json(self) -> JsonValue:
        return JSON.validate_json(self._body)

    async def aiter_lines(self) -> AsyncIterator[str]:
        for line in self._body.splitlines():
            yield line


class _Handler:
    def __init__(self, replies: list[Reply]) -> None:
        self._replies = replies
        self.requests: list[dict[str, JsonValue]] = []

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
            return _Response(DEFAULT_STATUS, "")
        reply = self._replies.pop(0) if len(self._replies) > 1 else self._replies[0]
        status = reply.get("status")
        body = reply.get("body")
        return _Response(
            DEFAULT_STATUS if status is None else status,
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


def config_for(provider: str) -> BaseSandboxConfig:
    config = ProviderConfigManager.get_provider_sandbox_config(SandboxProviders(provider))
    if config is None:
        raise ValueError(f"Code execution is not supported for provider: {provider}")
    return config


def handler_for(input_value: Input) -> _Handler:
    replies = input_value.get("replies")
    return _Handler([] if replies is None else replies)


def handle_value(handle: ContainerHandle) -> JsonValue:
    return JSON.validate_python(
        {"id": handle.id, "provider": handle.provider, "domain": handle.domain}
    )


def create_sandbox(
    config: BaseSandboxConfig, input_value: Input, handler: _Handler
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


def registry_answer(input_value: Input) -> JsonValue:
    tools = input_value.get("tools")
    names = input_value.get("names")
    if tools is None or names is None:
        raise ValueError("the registry operation needs tools and names")
    register_sandbox_tools(tools)
    resolved: dict[str, JsonValue] = {}
    for name in names:
        tool: dict[str, JsonValue] | None = resolve_sandbox_tool(name)
        resolved[name] = tool
    return JSON.validate_python({"resolved": resolved})


def create_answer(input_value: Input) -> JsonValue:
    provider = input_value.get("provider")
    if provider is None:
        raise ValueError("the create operation needs a provider")
    handler = handler_for(input_value)
    handle = create_sandbox(config_for(provider), input_value, handler)
    return JSON.validate_python(
        {"requests": handler.requests, "handle": handle_value(handle)}
    )


def create_and_run_answer(input_value: Input) -> JsonValue:
    provider = input_value.get("provider")
    code = input_value.get("code")
    if provider is None or code is None:
        raise ValueError("the create_and_run operation needs a provider and code")
    handler = handler_for(input_value)
    config = config_for(provider)
    handle = create_sandbox(config, input_value, handler)
    language = input_value.get("language")
    result: CodeExecutionResult = asyncio.run(
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
    return JSON.validate_python(
        {
            "requests": handler.requests,
            "handle": handle_value(handle),
            "result": JSON.validate_json(result.model_dump_json()),
        }
    )


def delete_answer(input_value: Input) -> JsonValue:
    provider = input_value.get("provider")
    container = input_value.get("container")
    if provider is None or container is None:
        raise ValueError("the delete operation needs a provider and a container id")
    handler = handler_for(input_value)
    deleted: bool = asyncio.run(
        config_for(provider).adelete_sandbox(
            container=container,
            api_key=input_value.get("api_key"),
            api_base=input_value.get("api_base"),
            client=handler,
        )
    )
    return JSON.validate_python({"requests": handler.requests, "deleted": deleted})


def run(input_value: Input) -> JsonValue:
    environment = input_value.get("environment")
    if environment is not None:
        for name, value in environment.items():
            os.environ[name] = value
    operation = input_value["operation"]
    if operation == "registry":
        return registry_answer(input_value)
    if operation == "create":
        return create_answer(input_value)
    if operation == "create_and_run":
        return create_and_run_answer(input_value)
    return delete_answer(input_value)
