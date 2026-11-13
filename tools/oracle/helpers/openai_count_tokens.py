import asyncio
import json
import os
from typing import TypeAlias, cast
from unittest import mock

import httpx
import litellm
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.llms.openai.common_utils import OpenAIError
from litellm.llms.openai.responses.count_tokens.handler import OpenAICountTokensHandler
from litellm.llms.openai.responses.count_tokens.token_counter import OpenAITokenCounter

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]

HANDLER_CLIENT = "litellm.llms.openai.responses.count_tokens.handler.get_async_httpx_client"
HOST = "openai.test"


class Provider:
    def __init__(self, status: int, body: Json) -> None:
        self.status = status
        self.body = body
        self.calls: list[Object] = []

    def handle(self, request: httpx.Request) -> httpx.Response:
        timeout = cast(dict[str, float], request.extensions["timeout"])
        self.calls.append(
            {
                "url": str(request.url),
                "path": request.url.path,
                "content_type": request.headers.get("content-type"),
                "authorization": request.headers.get("authorization"),
                "timeout": int(timeout["read"]),
                "body": cast(Json, json.loads(request.content)),
                "deployment_base": request.url.host == HOST,
            }
        )
        return httpx.Response(
            self.status,
            content=json.dumps(self.body, separators=(",", ":")).encode(),
            headers={"content-type": "application/json"},
        )

    def client(self) -> AsyncHTTPHandler:
        handler = AsyncHTTPHandler()
        handler.client = httpx.AsyncClient(transport=httpx.MockTransport(self.handle))
        return handler


def text(fields: Object, name: str) -> str | None:
    value = fields.get(name)
    if value is not None and not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def optional_list(fields: Object, name: str) -> list[dict[str, object]] | None:
    return cast("list[dict[str, object]] | None", fields.get(name))


def environment(fields: Object) -> dict[str, str]:
    raw = cast(dict[str, str], fields.get("env", {}))
    return dict(raw)


def key_source(call: Object, fields: Object) -> str | None:
    authorization = call["authorization"]
    if authorization is None:
        return None
    env_key = environment(fields).get("OPENAI_API_KEY")
    for name, key in (("api_key", text(fields, "api_key")), ("env", env_key)):
        if key is not None and authorization == f"Bearer {key}":
            return name
    return "other"


def count(fields: Object) -> Json:
    status = cast(int, fields.get("status", 200))
    default_body: Json = {"input_tokens": 7}
    provider = Provider(status, fields.get("body", default_body))
    base_path = text(fields, "base_path") or "/v1"
    overrides = cast(Object, fields.get("module_overrides") or {})
    deployment: dict[str, object] = {"litellm_params": {"api_base": f"http://{HOST}{base_path}"}}
    litellm_params = cast(dict[str, object], deployment["litellm_params"])
    if text(fields, "api_key") is not None:
        litellm_params["api_key"] = text(fields, "api_key")
    counter = OpenAITokenCounter()
    with (
        mock.patch.dict(os.environ, environment(fields), clear=True),
        mock.patch(HANDLER_CLIENT, return_value=provider.client()),
        mock.patch.object(litellm, "api_key", text(overrides, "api_key")),
        mock.patch.object(litellm, "openai_key", text(overrides, "openai_key")),
        mock.patch.object(litellm, "api_base", text(overrides, "api_base")),
    ):
        result = asyncio.run(
            counter.count_tokens(
                model_to_use=cast(str, fields["model"]),
                messages=cast("list[dict[str, object]]", fields["messages"]),
                contents=None,
                deployment=deployment,
                request_model="requested",
                tools=optional_list(fields, "tools"),
                system=fields.get("system"),
            )
        )
    call = provider.calls[0] if provider.calls else None
    request = None
    if call is not None:
        names = ("path", "content_type", "body", "deployment_base")
        request = {name: call[name] for name in names}
        request["authorization_source"] = key_source(call, fields)
    return {
        "selected": counter.should_use_token_counting_api(text(fields, "provider")),
        "result": None if result is None else cast(Json, result.model_dump()),
        "request": request,
    }


def prepare(fields: Object) -> Json:
    provider = Provider(200, '{"input_tokens":1}')
    timeout = cast(int | None, fields.get("timeout"))
    handler = OpenAICountTokensHandler()
    try:
        with (
            mock.patch.dict(os.environ, {}, clear=True),
            mock.patch(HANDLER_CLIENT, return_value=provider.client()),
        ):
            asyncio.run(
                handler.handle_count_tokens_request(
                    model=cast(str, fields["model"]),
                    input=cast(str, fields["input"]),
                    api_key=cast(str, fields["api_key"]),
                    api_base=text(fields, "api_base"),
                    timeout=None if timeout is None else float(timeout),
                    tools=optional_list(fields, "tools"),
                    instructions=text(fields, "instructions"),
                )
            )
    except OpenAIError as error:
        return {"error": {"status": error.status_code, "message": error.message}}
    call = provider.calls[0]
    names = ("url", "content_type", "authorization", "timeout")
    return {"request": {name: call[name] for name in names}}


def run(input_value: dict[str, Json]) -> Json:
    [(operation, request)] = input_value.items()
    fields = cast(Object, request)
    match operation:
        case "count":
            return count(fields)
        case "prepare":
            return prepare(fields)
        case _:
            raise ValueError(f"unsupported operation {operation}")
