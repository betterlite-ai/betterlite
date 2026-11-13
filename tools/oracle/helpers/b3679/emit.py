import asyncio
from collections.abc import Sized
from typing import Final, Literal, TypedDict

from fastapi.exceptions import HTTPException
from litellm.proxy.guardrails.guardrail_hooks.unified_guardrail.unified_guardrail import (
    UnifiedLLMGuardrails,
)
from pydantic import JsonValue as Json
from pydantic import TypeAdapter

from .chunks import Hooks

GUARDRAILS: Final[UnifiedLLMGuardrails] = Hooks()


class TranslationScript(TypedDict):
    error_items: list[Json] | None


class EmissionCall(TypedDict):
    function: Literal["http_error_emission"]
    call_type: str | None
    status_code: int
    detail: Json
    responses_so_far: list[Json]
    request_data: dict[str, Json]
    stream_started: bool
    responses_yielded: list[Json] | None
    translation: TranslationScript | None


class ScriptedTranslation:
    def __init__(self, items: list[Json] | None) -> None:
        self.items = items

    def build_stream_error_items(
        self, exc: HTTPException, responses_so_far: Sized | None = None
    ) -> list[Json] | None:
        del exc
        if self.items is None:
            return None
        count = 0 if responses_so_far is None else len(responses_so_far)
        return [*self.items, f"items={count}"]


async def collect(call: EmissionCall) -> list[Json]:
    translation = (
        None
        if call["translation"] is None
        else ScriptedTranslation(call["translation"]["error_items"])
    )
    error = HTTPException(status_code=call["status_code"], detail=call["detail"])
    return [
        TypeAdapter(Json).validate_python(item)
        async for item in GUARDRAILS.emit_streaming_http_error(  # pyright: ignore[reportArgumentType, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/unified_guardrail/unified_guardrail.py#L438
            error,
            call["call_type"],
            call["responses_so_far"],
            call["request_data"],
            endpoint_translation=translation,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/unified_guardrail/unified_guardrail.py#L438
            stream_started=call["stream_started"],
            responses_yielded=call["responses_yielded"],
        )
    ]


def emission(call: EmissionCall) -> Json:
    try:
        return {"items": asyncio.run(collect(call))}
    except HTTPException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/unified_guardrail/unified_guardrail.py#L438
        return {
            "error": {
                "status_code": error.status_code,
                "detail": TypeAdapter(Json).validate_python(error.detail),  # pyright: ignore[reportAny]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/unified_guardrail/unified_guardrail.py#L438
            }
        }
