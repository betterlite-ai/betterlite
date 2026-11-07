import asyncio
import json
from collections.abc import AsyncIterator
from typing import TypeAlias, cast

import httpx
from litellm.constants import OPEN_SANDBOX_DEFAULT_LANGUAGE
from litellm.llms.base_llm.sandbox.transformation import ContainerHandle
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.llms.opensandbox.sandbox.transformation import OpenSandboxSandboxConfig

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class _Response:
    def __init__(self, status: int, body: str) -> None:
        self.status_code = status
        self._body = body

    def json(self) -> Json:
        return cast(Json, json.loads(self._body))

    async def aiter_lines(self) -> AsyncIterator[str]:
        for line in self._body.splitlines():
            yield line


class _Handler:
    def __init__(self, responses: list[tuple[int, str]]) -> None:
        self._responses = responses
        self.requests: list[dict[str, Json]] = []

    def _next(self) -> _Response:
        if not self._responses:
            return _Response(200, "")
        if len(self._responses) > 1:
            status, body = self._responses.pop(0)
        else:
            status, body = self._responses[0]
        return _Response(status, body)

    def _record(
        self,
        method: str,
        url: str,
        headers: dict[str, str],
        body: Json,
        timeout: httpx.Timeout | None,
    ) -> None:
        self.requests.append(
            {
                "method": method,
                "url": url,
                "headers": {name.lower(): value for name, value in headers.items()},
                "json": body,
                "timeout": _timeout_fields(timeout),
            }
        )

    async def get(
        self,
        url: str,
        headers: dict[str, str],
        params: dict[str, bool] | None = None,
    ) -> _Response:
        recorded = url if params is None else str(httpx.URL(url, params=params))
        self._record("GET", recorded, headers, None, None)
        return self._next()

    async def post(
        self,
        url: str,
        headers: dict[str, str],
        json: dict[str, Json],
        *,
        timeout: httpx.Timeout | None = None,  # noqa: ASYNC109
        stream: bool = False,
    ) -> _Response:
        del stream
        self._record("POST", url, headers, json, timeout)
        response = self._next()
        _raise_for_status("POST", url, response)
        return response

    async def delete(self, url: str, headers: dict[str, str]) -> _Response:
        self._record("DELETE", url, headers, None, None)
        response = self._next()
        _raise_for_status("DELETE", url, response)
        return response


def _raise_for_status(method: str, url: str, response: _Response) -> None:
    if httpx.codes.is_success(response.status_code):
        return
    request = httpx.Request(method, url)
    raise httpx.HTTPStatusError(
        "the API answered with an error status",
        request=request,
        response=httpx.Response(response.status_code, request=request),
    )


def _timeout_fields(timeout: httpx.Timeout | None) -> dict[str, Json]:
    if timeout is None:
        return {"connect": None, "read": None, "write": None, "pool": None}
    return {
        "connect": None if timeout.connect is None else float(timeout.connect),
        "read": None if timeout.read is None else float(timeout.read),
        "write": None if timeout.write is None else float(timeout.write),
        "pool": None if timeout.pool is None else float(timeout.pool),
    }


def _optional_string(input_value: dict[str, Json], name: str) -> str | None:
    value = input_value.get(name)
    return None if value is None else cast(str, value)


def _optional_bool(input_value: dict[str, Json], name: str) -> bool | None:
    value = input_value.get(name)
    return None if value is None else cast(bool, value)


def _optional_float(input_value: dict[str, Json], name: str) -> float | None:
    value = input_value.get(name)
    return None if value is None else float(cast(float, value))


def _optional_map(input_value: dict[str, Json], name: str) -> dict[str, str] | None:
    value = input_value.get(name)
    if value is None:
        return None
    return {key: cast(str, item) for key, item in cast(dict[str, Json], value).items()}


def _required_string(input_value: dict[str, Json], name: str) -> str:
    return cast(str, input_value[name])


def _language(input_value: dict[str, Json]) -> str:
    value = input_value.get("language")
    return OPEN_SANDBOX_DEFAULT_LANGUAGE if value is None else cast(str, value)


def _scripted(input_value: dict[str, Json]) -> list[tuple[int, str]]:
    responses = input_value.get("responses")
    if responses is None:
        return [(200, "")]
    return [
        (cast(int, entry.get("status", 200)), cast(str, entry.get("body", "")))
        for entry in cast(list[dict[str, Json]], responses)
    ]


def _container(input_value: dict[str, Json]) -> ContainerHandle | str:
    container = input_value.get("container")
    if isinstance(container, str):
        return container
    if not isinstance(container, dict):
        raise ValueError("the call needs a container handle or a sandbox id")
    handle = ContainerHandle(
        id=cast(str, container["id"]),
        provider=cast(str, container.get("provider", "opensandbox")),
        domain=_optional_string(container, "domain"),
    )
    hidden = _optional_map(container, "hidden_params")
    handle._hidden_params = cast(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        dict[str, Json], {} if hidden is None else dict(hidden)
    )
    return handle


def _handle_output(handle: ContainerHandle) -> Json:
    hidden: dict[str, Json] = cast(  # pyright: ignore[reportPrivateUsage]
        dict[str, Json],
        handle._hidden_params,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    )
    return {
        "id": handle.id,
        "provider": handle.provider,
        "domain": handle.domain,
        "hidden_params": {
            "api_base": hidden.get("api_base"),
            "api_key": hidden.get("api_key"),
            "execd_endpoint": hidden.get("execd_endpoint"),
            "execd_headers": hidden.get("execd_headers"),
            "use_server_proxy": hidden.get("use_server_proxy"),
        },
    }


def run(input_value: dict[str, Json]) -> Json:
    operation = cast(str, input_value["operation"])
    config = OpenSandboxSandboxConfig()
    handler = _Handler(_scripted(input_value))
    client = cast(AsyncHTTPHandler, handler)
    if operation == "create":
        handle = asyncio.run(
            config.acreate_sandbox(  # pyright: ignore[reportUnknownMemberType]
                template=_optional_string(input_value, "template"),
                timeout=cast("int | None", input_value.get("timeout")),
                allow_internet_access=_optional_bool(input_value, "allow_internet_access"),
                api_key=_optional_string(input_value, "api_key"),
                api_base=_optional_string(input_value, "api_base"),
                metadata=_optional_map(input_value, "metadata"),
                env_vars=_optional_map(input_value, "env_vars"),
                resource_limits=_optional_map(input_value, "resource_limits"),
                resource_requests=_optional_map(input_value, "resource_requests"),
                entrypoint=cast("list[str] | None", input_value.get("entrypoint")),
                network_policy=cast("dict[str, object] | None", input_value.get("network_policy")),
                secure_access=cast(bool, input_value.get("secure_access", False)),
                use_server_proxy=cast(bool, input_value.get("use_server_proxy", False)),
                ready_timeout=_optional_float(input_value, "ready_timeout"),
                poll_interval=_optional_float(input_value, "poll_interval"),
                client=client,
            )
        )
        return {"handle": _handle_output(handle), "requests": cast(Json, handler.requests)}
    if operation == "run":
        result = asyncio.run(
            config.arun_code(  # pyright: ignore[reportUnknownMemberType]
                container=_container(input_value),
                code=_required_string(input_value, "code"),
                api_key=_optional_string(input_value, "api_key"),
                api_base=_optional_string(input_value, "api_base"),
                language=_language(input_value),
                use_server_proxy=cast(bool, input_value.get("use_server_proxy", False)),
                ready_timeout=_optional_float(input_value, "ready_timeout"),
                poll_interval=_optional_float(input_value, "poll_interval"),
                client=client,
            )
        )
        return {
            "requests": cast(Json, handler.requests),
            "result": cast(Json, result.model_dump()),
        }
    if operation == "delete":
        deleted = asyncio.run(
            config.adelete_sandbox(  # pyright: ignore[reportUnknownMemberType]
                container=_container(input_value),
                api_key=_optional_string(input_value, "api_key"),
                api_base=_optional_string(input_value, "api_base"),
                client=client,
            )
        )
        return {"deleted": deleted, "requests": cast(Json, handler.requests)}
    raise ValueError(f"unknown operation {operation}")
