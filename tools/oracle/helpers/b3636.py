from datetime import UTC, datetime
from typing import Literal, TypedDict

from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.proxy.pass_through_endpoints.llm_provider_handlers.openai_passthrough_logging_handler import (
    OpenAIPassthroughLoggingHandler,
    count_relayed_prompt_tokens,
)
from litellm.utils import ModelResponse, TextCompletionResponse
from pydantic import JsonValue as Json, TypeAdapter


class AssembleInput(TypedDict):
    function: Literal["assemble"]
    model: str
    chunks: list[str]
    messages: list[dict[str, Json]] | None


class CountInput(TypedDict):
    function: Literal["count"]
    model: str
    messages: list[dict[str, Json]] | None


class DumpMessage(TypedDict):
    role: str | None
    content: str | None


class DumpChoice(TypedDict):
    index: int
    finish_reason: str | None
    message: DumpMessage


class DumpUsage(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class Dump(TypedDict):
    id: str
    object: str
    created: int
    model: str | None
    system_fingerprint: str | None
    choices: list[DumpChoice]
    usage: DumpUsage | None


class ChoiceView(TypedDict):
    index: int
    finish_reason: str | None
    role: str | None
    content: str | None


class UsageView(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class ResponseView(TypedDict):
    id: str
    object: str
    created: int
    model: str | None
    system_fingerprint: str | None
    choices: list[ChoiceView]
    usage: UsageView | None


class CountAnswer(TypedDict):
    prompt_tokens: int


Input = AssembleInput | CountInput
DUMP: TypeAdapter[Dump] = TypeAdapter(Dump)


def logging_obj(model: str) -> Logging:
    return Logging(
        model=model,
        messages=None,
        stream=True,
        call_type="acompletion",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="b3636",
        function_id="b3636",
    )


def response_view(response: ModelResponse | TextCompletionResponse) -> ResponseView:
    dump = DUMP.validate_json(response.model_dump_json())
    choices: list[ChoiceView] = []
    for choice in dump["choices"]:
        message = choice["message"]
        choices.append(
            {
                "index": choice["index"],
                "finish_reason": choice["finish_reason"],
                "role": message["role"],
                "content": message["content"],
            }
        )
    usage = dump["usage"]
    billed: UsageView | None = None
    if usage is not None:
        billed = {
            "prompt_tokens": usage["prompt_tokens"],
            "completion_tokens": usage["completion_tokens"],
            "total_tokens": usage["total_tokens"],
        }
    return {
        "id": dump["id"],
        "object": dump["object"],
        "created": dump["created"],
        "model": dump["model"],
        "system_fingerprint": dump["system_fingerprint"],
        "choices": choices,
        "usage": billed,
    }


def assemble(input_value: AssembleInput) -> ResponseView | None:
    response = OpenAIPassthroughLoggingHandler()._build_complete_streaming_response(  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_provider_handlers/openai_passthrough_logging_handler.py#L552-L611
        all_chunks=input_value["chunks"],
        litellm_logging_obj=logging_obj(input_value["model"]),
        model=input_value["model"],
        messages=input_value["messages"],
    )
    if response is None:
        return None
    return response_view(response)


def run(input_value: Input) -> ResponseView | CountAnswer | None:
    if input_value["function"] == "assemble":
        return assemble(input_value)
    counted = count_relayed_prompt_tokens(input_value["model"], input_value["messages"])
    return {"prompt_tokens": counted}
