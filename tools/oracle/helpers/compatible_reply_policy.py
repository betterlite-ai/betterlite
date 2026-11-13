import json
from typing import Literal, TypeAlias, cast
from unittest.mock import MagicMock

import httpx
from litellm import Choices, ModelResponse, Usage
from litellm.llms.openai_like.chat.transformation import OpenAILikeChatConfig
from litellm.llms.openai_like.common_utils import OpenAILikeBase, OpenAILikeError
from litellm.llms.openai_like.embedding.handler import OpenAILikeEmbeddingHandler

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]


class Recorder:
    def __init__(self) -> None:
        self.body = ""

    def post(self, url: str, headers: dict[str, str], data: str) -> httpx.Response:  # noqa: ARG002
        self.body = data
        reply: Object = {"object": "list", "data": [], "model": "m", "usage": {"prompt_tokens": 0}}
        return httpx.Response(200, json=reply, request=httpx.Request("POST", url))


def validate(call: Object) -> Json:
    base = OpenAILikeBase()
    try:
        url, sent = base._validate_environment(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]
            cast(str | None, call.get("api_key")),
            cast(str | None, call.get("api_base")),
            cast(Literal["chat_completions", "embeddings"], call["endpoint"]),
            cast(dict[str, str] | None, call.get("headers")),
            cast(bool | None, call.get("custom_endpoint")),
        )
    except OpenAILikeError as error:
        return {"status": error.status_code, "message": error.message}
    return {"url": url, "headers": cast(Json, sent)}


def transform(call: Object) -> Json:
    response = OpenAILikeChatConfig._transform_response(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
        model="m",
        response=httpx.Response(200, json=call["reply"]),
        model_response=ModelResponse(),
        stream=False,
        logging_obj=MagicMock(),
        optional_params={},
        api_key=None,
        data={},
        messages=[],
        print_verbose=None,
        encoding=None,
        json_mode=cast(bool | None, call.get("json_mode")),
        custom_llm_provider=cast(str | None, call.get("provider")),
        base_model=cast(str | None, call.get("base_model")),
    )
    usage = cast(Usage, response.usage)  # pyright: ignore[reportAttributeAccessIssue]
    hidden = response._hidden_params  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType, reportUnknownMemberType]
    return {
        "model": response.model,
        "usage": [usage.prompt_tokens, usage.completion_tokens, usage.total_tokens],
        "choices": [
            {
                "content": choice.message.content,
                "tool_calls": None
                if choice.message.tool_calls is None
                else len(choice.message.tool_calls),
            }
            for choice in cast(list[Choices], response.choices)  # pyright: ignore[reportUnnecessaryCast]
        ],
        "hidden_model": cast(Json, hidden.get("model")),  # pyright: ignore[reportUnknownMemberType]
    }


def options(call: Object) -> Json:
    recorder = Recorder()
    OpenAILikeEmbeddingHandler().embedding(  # pyright: ignore[reportUnknownMemberType]
        model=cast(str, call["model"]),
        input=cast(list[str], call["input"]),
        timeout=5.0,
        logging_obj=MagicMock(),
        api_key="key",
        api_base="https://api.test",
        optional_params=cast(Object, call["options"]),
        client=recorder,
    )
    return cast(Json, json.loads(recorder.body))


def run(value: Object) -> Json:
    handlers = {"validate": validate, "transform": transform, "options": options}
    return {
        name: [handler(call) for call in cast(list[Object], value.get(name) or [])]
        for name, handler in handlers.items()
    }
