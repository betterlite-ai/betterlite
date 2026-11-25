from typing import TypedDict, cast

import litellm
from litellm.proxy.pass_through_endpoints.llm_provider_handlers import (
    anthropic_passthrough_logging_handler as handler_module,
)
from litellm.types.utils import (
    Choices,
    CompletionTokensDetailsWrapper,
    Message,
    ModelResponse,
    Usage,
)
from pydantic import JsonValue

Handler = handler_module.AnthropicPassthroughLoggingHandler


class TailCall(TypedDict):
    chunks: list[str]


class RecoverCall(TypedDict):
    model: str
    count: int
    content: str | None
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int
    has_details: bool
    text_tokens: int | None
    chunks: list[str]


class ClearCall(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int
    has_details: bool
    text_tokens: int | None
    cost: str | None
    hidden_response_cost: str | None


class Input(TypedDict):
    tails: list[TailCall]
    recovers: list[RecoverCall]
    clears: list[ClearCall]


class RecoverAnswer(TypedDict):
    changed: bool
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int
    text_tokens: int | None


class ClearAnswer(TypedDict):
    cost_cleared: bool
    hidden_cost_cleared: bool


class Answer(TypedDict):
    tails: list[bool]
    recovers: list[RecoverAnswer]
    clears: list[ClearAnswer]


class FixedCounter:
    def __init__(self, count: int) -> None:
        self.count = count

    def __call__(self, **_params: JsonValue) -> int:
        return self.count


def tail_answer(call: TailCall) -> bool:
    chunks = [bytes.fromhex(chunk) for chunk in call["chunks"]]
    return Handler._stream_was_interrupted(chunks)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_provider_handlers/anthropic_passthrough_logging_handler.py#L330-L361


def recover_answer(call: RecoverCall) -> RecoverAnswer:
    details = (
        CompletionTokensDetailsWrapper(text_tokens=call["text_tokens"])
        if call["has_details"]
        else None
    )
    response = ModelResponse(
        id="chatcmpl-b3632",
        choices=[  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2181-L2258
            Choices(
                index=0,
                message=Message(content=call["content"]),
                finish_reason="stop",
            ),
        ],
        created=1700000000,
        model=call["model"],
        usage=Usage(
            prompt_tokens=call["prompt_tokens"],
            completion_tokens=call["completion_tokens"],
            total_tokens=call["total_tokens"],
            completion_tokens_details=details,
        ),
    )
    chunks = [bytes.fromhex(chunk) for chunk in call["chunks"]]
    litellm.token_counter = FixedCounter(call["count"])
    recovered = Handler._recover_interrupted_stream_output_tokens(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_provider_handlers/anthropic_passthrough_logging_handler.py#L363-L403
        response=response,
        all_chunks=chunks,
        model=call["model"],
    )
    usage = cast(Usage, response.usage)  # type: ignore[attr-defined]  # pyright: ignore[reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2181-L2258
    details_after = usage.completion_tokens_details
    return {
        "changed": recovered is not None,
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
        "text_tokens": None if details_after is None else details_after.text_tokens,
    }


def clear_answer(call: ClearCall) -> ClearAnswer:
    details = (
        CompletionTokensDetailsWrapper(text_tokens=call["text_tokens"])
        if call["has_details"]
        else None
    )
    cost = None if call["cost"] is None else float(call["cost"])
    hidden_cost = call["hidden_response_cost"]
    response = ModelResponse(
        id="chatcmpl-b3632",
        choices=[  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2181-L2258
            Choices(index=0, message=Message(content=None), finish_reason="stop"),
        ],
        created=1700000000,
        model="claude-test",
        usage=Usage(
            prompt_tokens=call["prompt_tokens"],
            completion_tokens=call["completion_tokens"],
            total_tokens=call["total_tokens"],
            completion_tokens_details=details,
            cost=cost,
        ),
    )
    hidden = cast(dict[str, JsonValue], response._hidden_params)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2088
    if hidden_cost is not None:
        hidden["response_cost"] = float(hidden_cost)
    usage = cast(Usage, response.usage)  # type: ignore[attr-defined]  # pyright: ignore[reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2181-L2258
    Handler._clear_placeholder_cost(response, usage)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_provider_handlers/anthropic_passthrough_logging_handler.py#L406-L408
    return {
        "cost_cleared": usage.cost is None,
        "hidden_cost_cleared": "response_cost" not in hidden,
    }


def run(input_value: Input) -> Answer:
    return {
        "tails": [tail_answer(call) for call in input_value["tails"]],
        "recovers": [recover_answer(call) for call in input_value["recovers"]],
        "clears": [clear_answer(call) for call in input_value["clears"]],
    }
