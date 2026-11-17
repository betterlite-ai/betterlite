import httpx
from litellm.llms.bedrock.common_utils import BedrockError
from pydantic import BaseModel, ConfigDict, JsonValue

from .wire import JSON, Case, Chunk, FakeFailure

class DeltaView(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    content: str | None = None
    role: str | None = None

class ChoiceView(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    delta: DeltaView
    finish_reason: str | None = None

class UsageView(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    prompt_tokens: int
    completion_tokens: int
    total_tokens: int

class ChunkView(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    choices: list[ChoiceView] | None = None
    text: str | None = None
    tool_use: JsonValue | None = None
    is_finished: bool | None = None
    finish_reason: str | None = None
    usage: UsageView | None = None
    index: int | None = None

class ResponseView(BaseModel):
    model_config = ConfigDict(from_attributes=True, arbitrary_types_allowed=True)

    headers: httpx.Headers

class ErrorView(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    status_code: int
    message: str
    response: ResponseView | None = None

class StatusView(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    status_code: int

def failure_status(error: BedrockError) -> JsonValue:
    view = StatusView.model_validate(error)
    payload: dict[str, JsonValue] = {"status": view.status_code}
    return JSON.validate_python(payload)

def chunk_payload(value: Chunk) -> JsonValue:
    view = ChunkView.model_validate(value)
    usage: JsonValue = None
    if view.usage is not None:
        usage = {
            "prompt_tokens": view.usage.prompt_tokens,
            "completion_tokens": view.usage.completion_tokens,
            "total_tokens": view.usage.total_tokens,
        }
    if view.choices is None:
        generic: dict[str, JsonValue] = {
            "kind": "generic",
            "text": view.text,
            "tool_use": view.tool_use,
            "is_finished": view.is_finished,
            "finish_reason": view.finish_reason,
            "usage": usage,
            "index": view.index,
        }
        return JSON.validate_python(generic)
    choice = view.choices[0]
    stream: dict[str, JsonValue] = {
        "kind": "stream",
        "content": choice.delta.content,
        "role": choice.delta.role,
        "finish_reason": choice.finish_reason,
        "usage": usage,
    }
    return JSON.validate_python(stream)

def provider_error(error: BedrockError, case: Case) -> JsonValue:
    view = ErrorView.model_validate(error)
    headers: JsonValue = None
    if case["compare"]["headers"]:
        pairs: list[list[str]] = []
        if view.response is not None:
            pairs = sorted([name, value] for name, value in view.response.headers.items())
        headers = JSON.validate_python(pairs)
    payload: dict[str, JsonValue] = {
        "kind": "provider",
        "status": view.status_code,
        "message": view.message if case["compare"]["message"] else None,
        "headers": headers,
        "category": None,
    }
    return JSON.validate_python(payload)

def raised_error(category: FakeFailure) -> JsonValue:
    payload: dict[str, JsonValue] = {
        "kind": "raised",
        "status": None,
        "message": None,
        "headers": None,
        "category": category,
    }
    return JSON.validate_python(payload)
