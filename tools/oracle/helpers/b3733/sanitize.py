import base64
from collections.abc import Awaitable
from typing import Final, Literal, Protocol, TypedDict, runtime_checkable

import httpx
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.proxy.guardrails.guardrail_hooks.model_armor.model_armor import (
    ModelArmorGuardrail,
)
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter
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
    body: JsonValue


@runtime_checkable
class Guardrail(Protocol):
    def make_model_armor_request(
        self,
        *,
        content: str | None = None,
        source: Literal["user_prompt", "model_response"] = "user_prompt",
        file_bytes: bytes | None = None,
        file_type: str | None = None,
    ) -> Awaitable[dict[str, JsonValue]]: ...


class GuardrailRecord(BaseModel):
    model_config = ConfigDict(arbitrary_types_allowed=True)
    guard: Guardrail


_BODY_ADAPTER: Final = TypeAdapter(JsonValue)


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


def build_guardrail(call: SanitizeCall, recorder: Recorder) -> Guardrail:
    guard = ModelArmorGuardrail(
        template_id=call["template_id"],
        project_id=call["project_id"],
        location=call["location"],
        api_endpoint=call["api_endpoint"],
        access_token_provider=TokenProvider(call["access_token"], call["resolved_project_id"]),
        async_handler=AsyncHTTPHandler(transport=recorder),
    )
    # LiteLLM leaves make_model_armor_request's request_data and reply as dict[Any, Any]
    return GuardrailRecord.model_validate({"guard": guard}).guard


async def send(guard: Guardrail, run: RequestRun) -> None:
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


async def sanitize(call: SanitizeCall) -> JsonValue:
    recorder = Recorder()
    guard = build_guardrail(call, recorder)
    for run in call["requests"]:
        await send(guard, run)
    requests: list[JsonValue] = [
        {"url": request["url"], "body": request["body"]} for request in recorder.requests
    ]
    return {"requests": requests}
