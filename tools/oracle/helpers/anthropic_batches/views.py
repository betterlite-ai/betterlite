from litellm.types.utils import ModelResponse
from pydantic import JsonValue

from .models import PROJECTION, ZERO_COMPLETION, ZERO_PROMPT, Usage


def usage_view(usage: Usage) -> JsonValue:
    prompt = usage.prompt_tokens_details
    completion = usage.completion_tokens_details
    if prompt is None:
        prompt = ZERO_PROMPT
    if completion is None:
        completion = ZERO_COMPLETION
    return {
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
        "prompt_text_tokens": prompt.text_tokens,
        "cached_tokens": prompt.cached_tokens,
        "cache_creation_tokens": prompt.cache_creation_tokens,
        "cache_write_tokens": prompt.cache_write_tokens,
        "completion_text_tokens": completion.text_tokens,
        "reasoning_tokens": completion.reasoning_tokens,
        "accepted_prediction_tokens": completion.accepted_prediction_tokens,
    }


def response_view(response: ModelResponse) -> JsonValue:
    view = PROJECTION.validate_json(response.model_dump_json())
    choice = view.choices[0]
    usage: JsonValue = None
    if view.usage is not None:
        usage = usage_view(view.usage)
    return {
        "id": view.id,
        "object": view.object,
        "created": view.created,
        "model": view.model,
        "content": choice.message.content,
        "finish_reason": choice.finish_reason,
        "usage": usage,
    }
