import os
from typing import TypeAlias, TypedDict, cast

import httpx
from litellm.llms.bedrock.common_utils import BedrockError
from litellm.types.utils import EmbeddingResponse
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

BODY: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)

HiddenMetadata: TypeAlias = dict[str, JsonValue]


def set_environment(environment: dict[str, str]) -> None:
    os.environ.clear()
    os.environ.update(environment)


class RecordPayload(TypedDict):
    object: str
    embedding: list[JsonValue] | str
    index: int


class DetailsPayload(TypedDict):
    image_count: int | None
    query_count: int | None


class UsagePayload(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int
    prompt_tokens_details: DetailsPayload | None


class ReplyPayload(TypedDict):
    object: str
    data: list[RecordPayload]
    model: str | None
    usage: UsagePayload


REPLY: TypeAdapter[ReplyPayload] = TypeAdapter(ReplyPayload)


def reply_value(response: EmbeddingResponse) -> dict[str, JsonValue]:
    payload = REPLY.validate_json(response.model_dump_json())
    usage_payload = payload["usage"]
    details = usage_payload["prompt_tokens_details"]
    details_value: JsonValue = None
    if details is not None:
        details_value = {
            "image_count": details["image_count"],
            "query_count": details["query_count"],
        }
    usage: dict[str, JsonValue] = {
        "prompt_tokens": usage_payload["prompt_tokens"],
        "completion_tokens": usage_payload["completion_tokens"],
        "total_tokens": usage_payload["total_tokens"],
        "prompt_tokens_details": details_value,
    }
    data: list[JsonValue] = [
        {"object": record["object"], "embedding": record["embedding"], "index": record["index"]}
        for record in payload["data"]
    ]
    hidden = cast(HiddenMetadata, response._hidden_params)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2700
    return {
        "object": payload["object"],
        "data": data,
        "model": payload["model"],
        "usage": usage,
        "invocation_arn": hidden.get("_invocation_arn"),
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


class FailureView(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    status_code: int
    status_code_is_synthesized: bool
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
        "request": request,
        "response": answer,
        "headers": headers,
        "body": view.body,
        "response_headers": response_headers,
    }
