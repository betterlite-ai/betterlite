import asyncio
from contextlib import aclosing
from typing import Final

import httpx
import litellm
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.guardrails.guardrail_hooks.model_armor.model_armor import (
    ModelArmorGuardrail,
)
from litellm.types.guardrails import GuardrailEventHooks
from pydantic import ConfigDict, JsonValue, TypeAdapter

from b3735.case import JSON, Hook, Input, build_items, feed, metadata_view, project

GUARDRAIL_NAME: Final = "model-armor"
TEMPLATE_ID: Final = "t"
PROJECT_ID: Final = "p"
LOCATION: Final = "us-central1"

ARBITRARY: Final = ConfigDict(arbitrary_types_allowed=True)


class TokenProvider:
    def __init__(self, access_token: str, project_id: str) -> None:
        self.access_token = access_token
        self.project_id = project_id

    async def __call__(self) -> tuple[str, str]:
        return (self.access_token, self.project_id)


class Recorder(httpx.MockTransport):
    def __init__(self, status: int, body: str) -> None:
        super().__init__(self.handle)
        self.status = status
        self.body = body
        self.requests: list[httpx.Request] = []

    def handle(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(request)
        return httpx.Response(self.status, text=self.body, request=request)


def scanned(recorder: Recorder) -> JsonValue:
    if not recorder.requests:
        return None
    body = JSON.validate_json(recorder.requests[0].content)
    match body:
        case {"modelResponseData": {"text": text}}:
            return text
        case None | bool() | int() | float() | str() | list() | dict():
            return None


async def run_case(hook: Hook, recorder: Recorder, request: Input) -> JsonValue:
    guardrails: list[JsonValue] = [GUARDRAIL_NAME]
    metadata: dict[str, JsonValue] = {"guardrails": guardrails}
    request_data: dict[str, JsonValue] = {"metadata": metadata}
    items = build_items(request)
    chunks: list[JsonValue] = []
    streamed = hook.async_post_call_streaming_iterator_hook(
        UserAPIKeyAuth(), feed(items), request_data
    )
    async with aclosing(streamed) as stream:
        async for item in stream:
            chunks.append(project(item))
    return {
        "items": chunks,
        "scanned": scanned(recorder),
        "metadata": metadata_view(request_data),
    }


def build_guardrail(request: Input, recorder: Recorder) -> ModelArmorGuardrail:
    return ModelArmorGuardrail(
        template_id=TEMPLATE_ID,
        project_id=PROJECT_ID,
        location=LOCATION,
        access_token_provider=TokenProvider("credential", PROJECT_ID),
        async_handler=AsyncHTTPHandler(transport=recorder),
        sanitize_error_detail=request["sanitize_error_detail"],
        guardrail_name=GUARDRAIL_NAME,
        event_hook=[GuardrailEventHooks.post_call],
        mask_response_content=request["mask_response_content"],
        fail_on_error=request["fail_on_error"],
    )


def as_hook(guardrail: ModelArmorGuardrail) -> Hook:
    adapter: Final[TypeAdapter[Hook]] = TypeAdapter[Hook](Hook, config=ARBITRARY)
    return adapter.validate_python(guardrail)


def run(input_value: Input) -> JsonValue:
    recorder = Recorder(input_value["answer_status"], input_value["answer_body"])
    guardrail = as_hook(build_guardrail(input_value, recorder))
    return asyncio.run(run_case(guardrail, recorder, input_value))


# The Anthropic replay's unknown-model banner would corrupt the helper's JSON on stdout.
litellm.suppress_debug_info = True
