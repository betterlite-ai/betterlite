from typing import cast

from litellm.llms.openai.chat.gpt_transformation import OpenAIGPTConfig
from litellm.llms.openai.common_utils import OpenAIError
from litellm.types.utils import ModelResponseStream

from . import ChunkStep, Json, Object
from .support import as_object


def run(steps: list[ChunkStep]) -> Json:
    handler = OpenAIGPTConfig().get_model_response_iterator(  # pyright: ignore[reportAny]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L816-L826
        iter([]), sync_stream=False, json_mode=False
    )  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L816-L826
    outputs: list[Json] = []
    for step in steps:
        try:
            parsed = handler.chunk_parser(step["chunk"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/base_model_iterator.py#L84-L92
        except OpenAIError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/common_utils.py#L57-L84
            outputs.append({"ok": False, "status": error.status_code})
        else:
            outputs.append({
                "ok": True,
                "chunk": chunk_view(cast(ModelResponseStream, parsed)),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2103-L2159
            })
    return outputs


def chunk_view(result: ModelResponseStream) -> Json:
    dumped = as_object(result.model_dump())  # type: ignore[misc, no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2103-L2159
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
