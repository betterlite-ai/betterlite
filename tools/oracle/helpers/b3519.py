import asyncio
import os
from typing import Final, Literal, Protocol, Required, TypedDict

import httpx
from litellm.constants import OPEN_SANDBOX_DEFAULT_LANGUAGE
from litellm.llms.base_llm.sandbox.transformation import (
    BaseSandboxConfig,
    CodeExecutionResult,
    ContainerHandle,
)
from litellm.sandbox import sandbox_tools
from litellm.types.utils import SandboxProviders
from litellm.utils import ProviderConfigManager
from pydantic import JsonValue, TypeAdapter

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
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


class ToolFields(TypedDict):
    sandbox_provider: str
    api_key: str | None
    api_base: str | None


class HandleFields(TypedDict):
    id: str
    provider: str
    domain: str | None


class ResultFields(TypedDict):
    stdout: str
    stderr: str
    results: list[dict[str, JsonValue]]
    error: dict[str, JsonValue] | None
    execution_count: int | None
    object: str


TOOL: Final[TypeAdapter[ToolFields]] = TypeAdapter(ToolFields)
HANDLE: Final[TypeAdapter[HandleFields]] = TypeAdapter(HandleFields)
RESULT: Final[TypeAdapter[ResultFields]] = TypeAdapter(ResultFields)


class _Response(httpx.Response):
    def __init__(self, status: int, body: str) -> None:
        super().__init__(status, content=body.encode())


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


class SandboxConfig(Protocol):
    async def acreate_sandbox(
        self,
        *,
        template: str | None = None,
        timeout: int | None = None,
        allow_internet_access: bool | None = None,
        api_key: str | None = None,
        api_base: str | None = None,
        metadata: dict[str, str] | None = None,
        env_vars: dict[str, str] | None = None,
        resource_limits: dict[str, str] | None = None,
        resource_requests: dict[str, str] | None = None,
        entrypoint: list[str] | None = None,
        network_policy: dict[str, JsonValue] | None = None,
        secure_access: bool | None = None,
        use_server_proxy: bool | None = None,
        ready_timeout: float | None = None,
        poll_interval: float | None = None,
        client: _Handler | None = None,
    ) -> ContainerHandle: ...

    async def arun_code(
        self,
        *,
        container: ContainerHandle | str,
        code: str,
        api_key: str | None = None,
        api_base: str | None = None,
        env_vars: dict[str, str] | None = None,
        language: str | None = None,
        use_server_proxy: bool | None = None,
        ready_timeout: float | None = None,
        poll_interval: float | None = None,
        client: _Handler | None = None,
    ) -> CodeExecutionResult: ...

    async def adelete_sandbox(
        self,
        *,
        container: ContainerHandle | str,
        api_key: str | None = None,
        api_base: str | None = None,
        client: _Handler | None = None,
    ) -> bool: ...


class SandboxToolRegistry(Protocol):
    def register_sandbox_tools(self, tools: list[dict[str, JsonValue]]) -> None: ...

    def resolve_sandbox_tool(self, name: str) -> dict[str, JsonValue] | None: ...


TOOLS: Final[SandboxToolRegistry] = sandbox_tools


def config_for(provider: str) -> SandboxConfig:
    config = ProviderConfigManager.get_provider_sandbox_config(SandboxProviders(provider))
    if config is None:
        raise ValueError(f"Code execution is not supported for provider: {provider}")
    return config


def handler_for(input_value: Input) -> _Handler:
    replies = input_value.get("replies")
    return _Handler([] if replies is None else replies)


def handle_value(handle: ContainerHandle) -> JsonValue:
    fields = HANDLE.validate_json(handle.model_dump_json())
    return JSON.validate_python(
        {"id": fields["id"], "provider": fields["provider"], "domain": fields["domain"]}
    )


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


def registry_answer(input_value: Input) -> JsonValue:
    tools = input_value.get("tools")
    names = input_value.get("names")
    if tools is None or names is None:
        raise ValueError("the registry operation needs tools and names")
    TOOLS.register_sandbox_tools(tools)
    resolved: dict[str, JsonValue] = {}
    for name in names:
        found = TOOLS.resolve_sandbox_tool(name)
        resolved[name] = None if found is None else TOOL.validate_python(found)
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
    fields = RESULT.validate_json(result.model_dump_json())
    return JSON.validate_python(
        {
            "requests": handler.requests,
            "handle": handle_value(handle),
            "result": fields,
        }
    )


def delete_answer(input_value: Input) -> JsonValue:
    provider = input_value.get("provider")
    container = input_value.get("container")
    if provider is None or container is None:
        raise ValueError("the delete operation needs a provider and a container id")
    handler = handler_for(input_value)
    deleted = asyncio.run(
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
