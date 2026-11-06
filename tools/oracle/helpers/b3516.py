import asyncio
import json
from collections.abc import AsyncIterator
from typing import TypeAlias, cast

import httpx
from litellm.llms.base_llm.sandbox.transformation import ContainerHandle
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.llms.e2b.sandbox.transformation import E2BSandboxConfig

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
_ERROR_STATUS = 400


class _Response:
    def __init__(self, status: int, body: str) -> None:
        self.status_code = status
        self._body = body

    def json(self) -> Json:
        return cast(Json, json.loads(self._body))

    async def aiter_lines(self) -> AsyncIterator[str]:
        for line in self._body.split("\n"):
            yield line


class _Handler:
    def __init__(self, status: int, body: str) -> None:
        self._status = status
        self._body = body
        self.requests: list[dict[str, Json]] = []

    def _answer(self, method: str, url: str, headers: dict[str, str], body: Json) -> _Response:
        self.requests.append(
            {
                "method": method,
                "url": url,
                "headers": {name.lower(): value for name, value in headers.items()},
                "json": body,
            }
        )
        if self._status >= _ERROR_STATUS:
            request = httpx.Request(method, url)
            raise httpx.HTTPStatusError(
                "the API answered with an error status",
                request=request,
                response=httpx.Response(self._status, request=request),
            )
        return _Response(self._status, self._body)

    async def post(
        self,
        url: str,
        headers: dict[str, str],
        json: dict[str, Json],
        *,
        stream: bool = False,
    ) -> _Response:
        del stream
        return self._answer("POST", url, headers, json)

    async def delete(self, url: str, headers: dict[str, str]) -> _Response:
        return self._answer("DELETE", url, headers, None)


def _optional_string(input_value: dict[str, Json], name: str) -> str | None:
    value = input_value.get(name)
    return None if value is None else cast(str, value)


def _optional_int(input_value: dict[str, Json], name: str) -> int | None:
    value = input_value.get(name)
    return None if value is None else cast(int, value)


def _optional_bool(input_value: dict[str, Json], name: str) -> bool | None:
    value = input_value.get(name)
    return None if value is None else cast(bool, value)


def _container(value: Json) -> ContainerHandle | str:
    if isinstance(value, str):
        return value
    fields = cast(dict[str, Json], value)
    domain = fields.get("domain")
    handle = ContainerHandle(
        id=cast(str, fields["id"]),
        provider="e2b",
        domain=None if domain is None else cast(str, domain),
    )
    handle._hidden_params = {  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        "envd_access_token": fields.get("envd_access_token"),
        "traffic_access_token": fields.get("traffic_access_token"),
        "api_key": fields.get("api_key"),
        "api_base": fields.get("api_base"),
    }
    return handle


def _env_vars(value: Json) -> dict[str, str] | None:
    if value is None:
        return None
    return {key: cast(str, item) for key, item in cast(dict[str, Json], value).items()}


def run(input_value: dict[str, Json]) -> Json:
    operation = cast(str, input_value["operation"])
    status = cast(int, input_value.get("response_status", 200))
    body = cast(str, input_value.get("response_body", ""))
    handler = _Handler(status, body)
    client = cast(AsyncHTTPHandler, handler)
    config = E2BSandboxConfig()
    if operation == "create":
        handle = asyncio.run(
            config.acreate_sandbox(  # pyright: ignore[reportUnknownMemberType]
                template=_optional_string(input_value, "template"),
                timeout=_optional_int(input_value, "timeout"),
                allow_internet_access=_optional_bool(input_value, "allow_internet_access"),
                api_key=_optional_string(input_value, "api_key"),
                api_base=_optional_string(input_value, "api_base"),
                client=client,
            )
        )
        return {
            "request": handler.requests[-1],
            "handle": {"id": handle.id, "provider": handle.provider, "domain": handle.domain},
        }
    container = _container(input_value.get("container"))
    if operation == "run":
        result = asyncio.run(
            config.arun_code(  # pyright: ignore[reportUnknownMemberType]
                container=container,
                code=cast(str, input_value["code"]),
                env_vars=_env_vars(input_value.get("env_vars")),
                client=client,
            )
        )
        return {"request": handler.requests[-1], "result": cast(Json, result.model_dump())}
    if operation == "delete":
        deleted = asyncio.run(
            config.adelete_sandbox(  # pyright: ignore[reportUnknownMemberType]
                container=container,
                api_key=_optional_string(input_value, "api_key"),
                api_base=_optional_string(input_value, "api_base"),
                client=client,
            )
        )
        return {"request": handler.requests[-1], "deleted": deleted}
    raise ValueError(f"unknown operation {operation}")
