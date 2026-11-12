from typing import cast
from unittest import mock

import litellm.types.utils
from litellm.llms.openai.chat.gpt_transformation import OpenAIGPTConfig
from litellm.llms.openai.common_utils import OpenAIError
from litellm.types.utils import ModelResponseStream

from . import ChunkStep, Json, Object
from .support import FixedClock, FixedUuid, as_object


def run(steps: list[ChunkStep]) -> Json:
    handler = OpenAIGPTConfig().get_model_response_iterator(iter([]), False, False)
    outputs: list[Json] = []
    with (
        mock.patch("litellm.types.utils.uuid.uuid4", new=FixedUuid()),
        mock.patch("litellm.types.utils.time.time", new=FixedClock()),
    ):
        for step in steps:
            try:
                parsed = handler.chunk_parser(step["chunk"])
            except OpenAIError as error:
                outputs.append({"ok": False, "status": error.status_code})
            else:
                outputs.append({"ok": True, "chunk": chunk_view(cast(ModelResponseStream, parsed))})  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/base_model_iterator.py#L84-L92
    return outputs


def chunk_view(result: ModelResponseStream) -> Json:
    dumped = as_object(result.model_dump())  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2103-L2159
    view: Object = {
        "id": dumped.get("id"),
        "object": dumped.get("object"),
        "created": dumped.get("created"),
        "model": dumped.get("model"),
        "choices": [choice_view(choice) for choice in cast(list[Json], dumped.get("choices"))],  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2006-L2038
    }
    usage = dumped.get("usage")
    if usage is not None:
        view["usage"] = usage_view(as_object(usage))
    tier = dumped.get("service_tier")
    if tier is not None:
        view["service_tier"] = tier
    return view


def choice_view(choice: Json) -> Json:
    fields = as_object(choice)
    delta = as_object(fields.get("delta"))
    return {
        "index": fields.get("index"),
        "finish_reason": fields.get("finish_reason"),
        "delta": delta_view(delta),
        "logprobs": fields.get("logprobs"),
    }


def delta_view(delta: Object) -> Json:
    view: Object = {
        name: delta.get(name)
        for name in ("role", "content", "function_call", "tool_calls", "audio")
    }
    reasoning = delta.get("reasoning_content")
    if reasoning is not None:
        view["reasoning_content"] = reasoning
    return view


def usage_view(usage: Object) -> Json:
    return {
        "prompt_tokens": usage.get("prompt_tokens"),
        "completion_tokens": usage.get("completion_tokens"),
        "total_tokens": usage.get("total_tokens"),
        "prompt_tokens_details": usage.get("prompt_tokens_details"),
        "completion_tokens_details": usage.get("completion_tokens_details"),
    }
