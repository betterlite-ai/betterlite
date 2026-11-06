import asyncio
import json
from collections.abc import AsyncIterator
from typing import TypeAlias, cast

import httpx
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
        request = httpx.Request("GET", "http://sandbox.invalid")
        if not httpx.codes.is_success(status):
            raise httpx.HTTPStatusError(
                "the API answered with an error status",
                request=request,
                response=httpx.Response(status, request=request),
            )
        return _Response(status, body)

    async def get(
        self,
        url: str,
        headers: dict[str, str],
        params: dict[str, bool] | None = None,
    ) -> _Response:
        self.requests.append(
            {
                "method": "GET",
                "url": url if params is None else str(httpx.URL(url, params=params)),
                "headers": {name.lower(): value for name, value in headers.items()},
                "json": None,
                "timeout": _timeout_fields(None),
            }
        )
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
        self.requests.append(
            {
                "method": "POST",
                "url": url,
                "headers": {name.lower(): value for name, value in headers.items()},
                "json": json,
                "timeout": _timeout_fields(timeout),
            }
        )
        return self._next()


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


def _optional_map(input_value: dict[str, Json], name: str) -> dict[str, str] | None:
    value = input_value.get(name)
    if value is None:
        return None
    return {key: cast(str, item) for key, item in cast(dict[str, Json], value).items()}


def _required_string(input_value: dict[str, Json], name: str) -> str:
    return cast(str, input_value[name])


def _headers(input_value: dict[str, Json]) -> dict[str, str]:
    headers = _optional_map(input_value, "headers")
    return {} if headers is None else headers


def _scripted(input_value: dict[str, Json]) -> list[tuple[int, str]]:
    responses = input_value.get("responses")
    if responses is None:
        body = _optional_string(input_value, "response_body")
        return [(200, "" if body is None else body)]
    return [
        (cast(int, entry.get("status", 200)), cast(str, entry.get("body", "")))
        for entry in cast(list[dict[str, Json]], responses)
    ]


def run(input_value: dict[str, Json]) -> Json:
    operation = cast(str, input_value["operation"])
    config = OpenSandboxSandboxConfig()
    if operation == "create_body":
        body = config._create_body(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            template=_optional_string(input_value, "template"),
            timeout=cast("int | None", input_value.get("timeout")),
            allow_internet_access=_optional_bool(input_value, "allow_internet_access"),
            metadata=_optional_map(input_value, "metadata"),
            env_vars=_optional_map(input_value, "env_vars"),
            resource_limits=_optional_map(input_value, "resource_limits"),
            resource_requests=_optional_map(input_value, "resource_requests"),
            entrypoint=cast("list[str] | None", input_value.get("entrypoint")),
            network_policy=cast("dict[str, object] | None", input_value.get("network_policy")),
            secure_access=cast(bool, input_value.get("secure_access", False)),
        )
        return cast(Json, body)
    handler = _Handler(_scripted(input_value))
    client = cast(AsyncHTTPHandler, handler)
    if operation == "wait_until_running":
        asyncio.run(
            config._wait_until_running(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                sandbox_id=_required_string(input_value, "sandbox_id"),
                api_base=_required_string(input_value, "api_base"),
                headers=_headers(input_value),
                client=client,
                ready_timeout=float(cast(float, input_value["ready_timeout"])),
                poll_interval=float(cast(float, input_value["poll_interval"])),
            )
        )
        return {"requests": cast(Json, handler.requests)}
    if operation == "wait_for_execd_endpoint":
        endpoint, endpoint_headers = asyncio.run(
            config._wait_for_execd_endpoint(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                sandbox_id=_required_string(input_value, "sandbox_id"),
                api_base=_required_string(input_value, "api_base"),
                headers=_headers(input_value),
                use_server_proxy=cast(bool, input_value.get("use_server_proxy", False)),
                client=client,
                ready_timeout=float(cast(float, input_value["ready_timeout"])),
                poll_interval=float(cast(float, input_value["poll_interval"])),
            )
        )
        return {
            "requests": cast(Json, handler.requests),
            "endpoint": endpoint,
            "headers": cast(Json, endpoint_headers),
        }
    if operation == "run":
        lines = asyncio.run(
            config._post_code(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                url=_required_string(input_value, "url"),
                headers=_headers(input_value),
                body=cast(dict[str, object], input_value["body"]),
                client=client,
            )
        )
        result = config._parse_lines(lines)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        return {
            "request": cast(Json, handler.requests[-1]),
            "result": cast(Json, result.model_dump()),
        }
    raise ValueError(f"unknown operation {operation}")
