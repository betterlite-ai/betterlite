import os

import httpx
from litellm.llms.bedrock.common_utils import BedrockError
from litellm.types.utils import EmbeddingResponse
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

BODY: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


def set_environment(environment: dict[str, str]) -> None:
    os.environ.clear()
    os.environ.update(environment)


class RecordView(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    object: str
    embedding: list[JsonValue] | str
    index: int


class DetailsView(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    image_count: int | None = None
    query_count: int | None = None


class UsageView(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int
    prompt_tokens_details: DetailsView | None


class HiddenView(BaseModel):
    model_config = ConfigDict(from_attributes=True, populate_by_name=True)
    invocation_arn: JsonValue = Field(default=None, alias="_invocation_arn")


class ReplyView(BaseModel):
    model_config = ConfigDict(from_attributes=True, populate_by_name=True)
    object: str
    data: list[RecordView]
    model: str | None
    usage: UsageView
    hidden: HiddenView = Field(alias="_hidden_params")


def reply_value(response: EmbeddingResponse) -> dict[str, JsonValue]:
    view = ReplyView.model_validate(response)
    details: dict[str, JsonValue] | None = None
    if view.usage.prompt_tokens_details is not None:
        details = {
            "image_count": view.usage.prompt_tokens_details.image_count,
            "query_count": view.usage.prompt_tokens_details.query_count,
        }
    usage: dict[str, JsonValue] = {
        "prompt_tokens": view.usage.prompt_tokens,
        "completion_tokens": view.usage.completion_tokens,
        "total_tokens": view.usage.total_tokens,
        "prompt_tokens_details": details,
    }
    data: list[JsonValue] = [
        {"object": record.object, "embedding": record.embedding, "index": record.index}
        for record in view.data
    ]
    return {
        "object": view.object,
        "data": data,
        "model": view.model,
        "usage": usage,
        "invocation_arn": view.hidden.invocation_arn,
    }


class RequestView(BaseModel):
    model_config = ConfigDict(from_attributes=True, arbitrary_types_allowed=True)
    method: str
    url: httpx.URL


class ResponseView(BaseModel):
    model_config = ConfigDict(from_attributes=True, arbitrary_types_allowed=True)
    status_code: int
    request: RequestView
    headers: httpx.Headers
    text: str


class FailureView(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    status_code: int
    status_code_is_synthesized: bool
    message: str
    request: RequestView
    response: ResponseView
    headers: dict[str, str] | None
    body: JsonValue


def failure_value(error: BedrockError, names: set[str]) -> dict[str, JsonValue]:
    view = FailureView.model_validate(error)
    headers: dict[str, JsonValue] | None = None
    if view.headers is not None:
        headers = {
            name.lower(): value
            for name, value in view.headers.items()
            if name.lower() in names
        }
    response_headers: dict[str, JsonValue] = {
        name.lower(): value
        for name, value in dict(view.response.headers).items()
        if name.lower() in names
    }
    request: dict[str, JsonValue] = {
        "method": view.request.method,
        "url": str(view.request.url),
    }
    answer: dict[str, JsonValue] = {
        "status_code": view.response.status_code,
        "request": {
            "method": view.response.request.method,
            "url": str(view.response.request.url),
        },
    }
    return {
        "kind": "error",
        "status_code": view.status_code,
        "status_code_is_synthesized": view.status_code_is_synthesized,
        "message": view.message,
        "request": request,
        "response": answer,
        "headers": headers,
        "body": view.body,
        "response_text": view.response.text,
        "response_headers": response_headers,
    }
