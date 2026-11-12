import time
from typing import Protocol, cast

import httpx
import litellm
from litellm.llms.ollama.completion.transformation import OllamaConfig
from litellm.types.utils import ChatCompletionMessageToolCall, ModelResponse

from .support import (
    UUID_PATTERN,
    FrozenClock,
    Json,
    ReplyCase,
    ReplyInput,
    UsageRecord,
    WordCounter,
    arguments_value,
    config_builder,
    content_value,
    dict_or_empty,
    usage_list,
)


class ReplyMessage(Protocol):
    role: str
    content: str | None
    tool_calls: list[ChatCompletionMessageToolCall] | None

    def get(self, key: str) -> str | None: ...


class ReplyChoice(Protocol):
    finish_reason: str
    message: ReplyMessage


class ReplyResult(Protocol):
    model: str | None
    created: int
    choices: list[ReplyChoice]
    usage: UsageRecord | None


def calls_value(calls: list[ChatCompletionMessageToolCall] | None) -> Json:
    if calls is None:
        return None
    return [
        {
            "id": UUID_PATTERN.sub("<uuid>", call.id),
            "type": call.type,
            "name": call.function.name,
            "arguments": arguments_value(call.function.arguments),
        }
        for call in calls
    ]


def reply_value(result: ReplyResult) -> Json:
    choice = result.choices[0]
    message = choice.message
    return {
        "model": result.model,
        "created": result.created,
        "finish_reason": choice.finish_reason,
        "role": message.role,
        "content": content_value(message.content),
        "reasoning_content": message.get("reasoning_content"),
        "tool_calls": calls_value(message.tool_calls),
        "usage": usage_list(result.usage),
    }


def reply_case(config: OllamaConfig, case: ReplyCase) -> Json:
    empty: dict[str, Json] = {}
    messages: list[dict[str, str]] = [{"role": "user", "content": "hi"}]
    try:
        result = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/ollama/completion/transformation.py#L276
            "ReplyResult",
            config.transform_response(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/ollama/completion/transformation.py#L276
                model=case["model"],
                raw_response=httpx.Response(
                    200,
                    content=case["body"].encode(),
                    request=httpx.Request("POST", "https://host"),
                ),
                model_response=ModelResponse(),
                logging_obj=None,  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/ollama/completion/transformation.py#L281
                request_data=dict_or_empty(case.get("request_data")),
                messages=messages,  # type: ignore[arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/ollama/completion/transformation.py#L283
                optional_params=empty,
                litellm_params=empty,
                encoding=None,
            ),
        )
    except (AttributeError, KeyError, TypeError, ValueError):
        return {"ok": False, "status": None}
    return {"ok": True, "reply": reply_value(result)}


def reply_cases(input_value: ReplyInput) -> Json:
    time.time = FrozenClock()  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/ollama/completion/transformation.py#L349
    litellm.encoding = WordCounter()  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/ollama/completion/transformation.py#L352
    config = config_builder({})
    found: list[Json] = [reply_case(config, case) for case in input_value["replies"]]
    return {"replies": found}
