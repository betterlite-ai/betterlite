from typing import cast

from litellm.types.utils import ModelResponseStream, Usage
from pydantic import JsonValue

from .types import CHUNK_JSON, ExistingUsage, FieldsJson, JsonObject


def usage_of(usage: ExistingUsage) -> Usage:
    return Usage(
        prompt_tokens=usage["prompt_tokens"],
        completion_tokens=usage["completion_tokens"],
        total_tokens=usage["total_tokens"],
    )


def usage_view(usage: JsonObject | None) -> JsonValue:
    if usage is None:
        return None
    return {
        "prompt_tokens": usage.get("prompt_tokens"),
        "completion_tokens": usage.get("completion_tokens"),
        "total_tokens": usage.get("total_tokens"),
    }


def fields_of(source: FieldsJson) -> JsonObject:
    return {
        "role": source.get("role"),
        "content": source.get("content"),
        "annotations": source.get("annotations"),
    }


def chunk_view(chunk: ModelResponseStream) -> JsonValue:
    dumped = CHUNK_JSON.validate_json(chunk.model_dump_json())
    first = dumped["choices"][0]
    hidden = cast(JsonObject, chunk._hidden_params)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure_ai/agents/handler.py#L655-L703
    delta = fields_of(first["delta"])
    choice: JsonObject = {
        "index": first.get("index"),
        "finish_reason": first.get("finish_reason"),
        "delta": delta,
    }
    choices: list[JsonValue] = [choice]
    view: JsonObject = {
        "id": dumped.get("id"),
        "created": dumped.get("created"),
        "model": dumped.get("model"),
        "object": dumped.get("object"),
        "choices": choices,
        "thread_id": hidden.get("thread_id"),
    }
    return view
