from typing import Final, Literal, TypedDict

from fastapi.exceptions import HTTPException
from pydantic import JsonValue as Json
from pydantic import TypeAdapter

from .chunks import Hooks, normalize, stream_item

GUARDRAILS: Final = Hooks()


class TransformChunkCall(TypedDict):
    function: Literal["transform_chunk"]
    reference_chunk: Json
    mutated: list[tuple[int, str]]
    emitted: list[tuple[int, str]]
    holdback: list[tuple[int, int]]
    reasons: list[tuple[int, str | None]]
    held: list[tuple[int, int]]
    is_final: bool


def run(call: TransformChunkCall) -> Json:
    emitted = dict(call["emitted"])
    held = dict(call["held"])
    try:
        chunk = GUARDRAILS._build_transform_chunk(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/unified_guardrail/unified_guardrail.py#L472
            reference_chunk=stream_item(call["reference_chunk"]),
            mutated_text_per_choice=dict(call["mutated"]),
            emitted_text_per_choice=emitted,
            holdback_per_choice=dict(call["holdback"]),
            finish_reason_per_choice=dict(call["reasons"]),
            held_chars_per_choice=held,
            is_final=call["is_final"],
        )
    except HTTPException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/unified_guardrail/unified_guardrail.py#L505
        return {
            "error": {
                "status_code": error.status_code,
                "detail": TypeAdapter(Json).validate_python(error.detail),  # pyright: ignore[reportAny]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/unified_guardrail/unified_guardrail.py#L505
            }
        }
    return {
        "chunk": None if chunk is None else normalize(chunk),
        "emitted": [[index, text] for index, text in emitted.items()],
        "held": [[index, count] for index, count in held.items()],
    }
