import os

os.environ["LITELLM_RUST"] = "false"

from collections.abc import Callable
from typing import TYPE_CHECKING, TypeAlias, cast
from unittest import mock

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.bedrock.chat.agentcore.transformation import AmazonAgentCoreConfig
from litellm.llms.bedrock.common_utils import BedrockError
from litellm.types.utils import ModelResponse, Usage
from pydantic import BaseModel, ConfigDict

from bedrock_settings import ErrorView

if TYPE_CHECKING:
    from litellm.types.llms.openai import AllMessageValues

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
ChoiceValue: TypeAlias = dict[str, object]
ENDPOINT = "https://bedrock-agentcore.example/runtime"


class Message(BaseModel):
    model_config = ConfigDict(extra="forbid")
    role: str
    content: str | None = None


class Counts(BaseModel):
    model_config = ConfigDict(extra="forbid")
    prompt: int
    completion: int


class TokenUsage(BaseModel):
    model_config = ConfigDict(extra="forbid")
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class SuppliedResponse(BaseModel):
    model_config = ConfigDict(extra="forbid")
    id: str
    object: str
    created: int
    model: str
    system_fingerprint: str | None = None
    choices: list[ChoiceValue] = []
    usage: TokenUsage | None = None


class Input(BaseModel):
    model_config = ConfigDict(extra="forbid")
    status: int
    headers: list[tuple[str, str]]
    body: str
    model: str
    messages: list[Message]
    content: str
    response: SuppliedResponse
    tokens: Counts | None = None


def _counter(counts: Counts | None, model: str) -> Callable[..., int]:
    def count(
        *,
        messages: object = None,
        text: str | None = None,
        count_response_tokens: bool = False,
        **kwargs: object,
    ) -> int:
        if kwargs.get("model") != model:
            raise ValueError("the token counter was called with another model")
        if counts is None:
            raise ValueError("fixture token counter failure")
        if messages is not None and text is None:
            return counts.prompt
        if messages is None and text is not None and count_response_tokens is True:
            return counts.completion
        raise ValueError("the token counter was called with an unexpected request")

    return count


def _project(response: ModelResponse) -> Json:
    choice = response.choices[0]
    usage = cast("Usage | None", getattr(response, "usage", None))
    return cast(
        Json,
        {
            "id": response.id,
            "object": response.object,
            "created": response.created,
            "model": response.model,
            "system_fingerprint": response.system_fingerprint,
            "choices": [
                {
                    "index": choice.index,
                    "finish_reason": choice.finish_reason,
                    "role": choice.message.role,
                    "content": choice.message.content,
                }
            ],
            "usage": None
            if usage is None
            else {
                "prompt_tokens": usage.prompt_tokens,
                "completion_tokens": usage.completion_tokens,
                "total_tokens": usage.total_tokens,
            },
        },
    )


def _error(error: BedrockError) -> Json:
    viewed = cast(ErrorView, error)
    headers = viewed.headers
    own_headers = (
        list(headers.multi_items()) if isinstance(headers, httpx.Headers) else headers
    )
    return cast(
        Json,
        {
            "error": {
                "status_code": viewed.status_code,
                "status_code_is_synthesized": viewed.status_code_is_synthesized,
                "headers": own_headers,
                "response_status": viewed.response.status_code,
            }
        },
    )


def run(argument: Json) -> Json:
    supplied = Input.model_validate(argument)
    raw_response = httpx.Response(
        supplied.status,
        content=supplied.body.encode(),
        headers=supplied.headers,
        request=httpx.Request("POST", ENDPOINT),
    )
    supplied_usage = supplied.response.usage
    model_response = ModelResponse(
        id=supplied.response.id,
        object=supplied.response.object,
        created=supplied.response.created,
        model=supplied.response.model,
        system_fingerprint=supplied.response.system_fingerprint,
        usage=None
        if supplied_usage is None
        else Usage(
            prompt_tokens=supplied_usage.prompt_tokens,
            completion_tokens=supplied_usage.completion_tokens,
            total_tokens=supplied_usage.total_tokens,
        ),
    )
    messages = cast(
        "list[AllMessageValues]",
        [message.model_dump() for message in supplied.messages],
    )
    counter = _counter(supplied.tokens, supplied.model)
    config = AmazonAgentCoreConfig()
    with mock.patch("litellm.utils.token_counter", counter):
        try:
            result = config.transform_response(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType, reportUnknownArgumentType]
                model=supplied.model,
                raw_response=raw_response,
                model_response=model_response,
                logging_obj=cast(Logging, None),
                request_data={},
                messages=messages,
                optional_params={},
                litellm_params={},
                encoding=None,
            )
        except BedrockError as error:
            return _error(error)
    return _project(result)
