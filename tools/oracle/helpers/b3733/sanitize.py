import base64
from typing import Final, Literal, TypedDict

import httpx
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.proxy.guardrails.guardrail_hooks.model_armor.model_armor import (
    ModelArmorGuardrail,
)
from pydantic import JsonValue as Json
from pydantic import TypeAdapter
from typing_extensions import override


class FilePayload(TypedDict):
    bytes_b64: str
    byte_data_type: str


class RequestRun(TypedDict):
    source: Literal["user_prompt", "model_response"]
    content: str | None
    file: FilePayload | None


class SanitizeCall(TypedDict):
    function: Literal["sanitize_request"]
    template_id: str | None
    project_id: str | None
    location: str | None
    api_endpoint: str | None
    resolved_project_id: str
    access_token: str
    requests: list[RequestRun]


class RecordedRequest(TypedDict):
    url: str
    body: Json


_BODY_ADAPTER: Final = TypeAdapter(Json)


class TokenProvider:
    def __init__(self, access_token: str, project_id: str) -> None:
        self.access_token = access_token
        self.project_id = project_id

    async def __call__(self) -> tuple[str, str]:
        return (self.access_token, self.project_id)


class Recorder(httpx.AsyncBaseTransport):
    def __init__(self) -> None:
        self.requests: list[RecordedRequest] = []

    @override
    async def handle_async_request(self, request: httpx.Request) -> httpx.Response:
        body = _BODY_ADAPTER.validate_json(request.content)
        self.requests.append(RecordedRequest(url=str(request.url), body=body))
        return httpx.Response(200, content=b"{}", request=request)


async def send(guard: ModelArmorGuardrail, run: RequestRun) -> None:
    file = run["file"]
    if file is not None:
        await guard.make_model_armor_request(
            source=run["source"],
            file_bytes=base64.b64decode(file["bytes_b64"]),
            file_type=file["byte_data_type"],
        )
        return
    content = run["content"]
    if content is None:
        raise ValueError("each request carries content or a file")
    await guard.make_model_armor_request(content=content, source=run["source"])


async def sanitize(call: SanitizeCall) -> Json:
    recorder = Recorder()
    guard = ModelArmorGuardrail(
        template_id=call["template_id"],
        project_id=call["project_id"],
        location=call["location"],
        api_endpoint=call["api_endpoint"],
        access_token_provider=TokenProvider(call["access_token"], call["resolved_project_id"]),
        async_handler=AsyncHTTPHandler(transport=recorder),
    )
    for run in call["requests"]:
        await send(guard, run)
    requests: list[Json] = [
        {"url": request["url"], "body": request["body"]} for request in recorder.requests
    ]
    return {"requests": requests}
