from collections.abc import Mapping
from typing import Final

from betterlite._endpoints import (
    CallPayload,
    CallValue,
    acall_native,
    call_native,
    read_settings,
    to_call_payload,
)
from betterlite._native import NativeTextError, NativeTextSession
from betterlite.types.llms.openai import OpenAIModerationResponse

SETTINGS: Final[tuple[str, ...]] = ("api_key", "openai_key")


def moderation(
    input: str,
    model: str | None = None,
    api_key: str | None = None,
    **kwargs: CallValue,
) -> OpenAIModerationResponse:
    values = dict(kwargs)
    values |= {"input": input, "model": model, "api_key": api_key}
    answer = call_native(
        NativeTextSession("{}").moderation, moderation_payload(values), NativeTextError
    )
    return OpenAIModerationResponse.model_validate(answer)


async def amoderation(
    input: str,
    model: str | None = None,
    api_key: str | None = None,
    custom_llm_provider: str | None = None,
    **kwargs: CallValue,
) -> OpenAIModerationResponse:
    # amoderation routes through get_llm_provider, which Rust repeats to strip the model prefix
    values: dict[str, CallValue] = {
        **kwargs,
        "input": input,
        "model": model,
        "api_key": api_key,
        "custom_llm_provider": custom_llm_provider,
    }
    answer = await acall_native(
        NativeTextSession("{}").amoderation, moderation_payload(values), NativeTextError
    )
    return OpenAIModerationResponse.model_validate(answer)


def moderation_payload(values: Mapping[str, CallValue]) -> CallPayload:
    # the top-level names are the payload's own: it drops the logger, refuses a client and warns
    payload: dict[str, CallValue] = {
        "arguments": forwarded_arguments(values),
        "settings": read_settings(SETTINGS),
        "client": values.get("client"),
        "logger_fn": values.get("logger_fn"),
        "shared_session": values.get("shared_session"),
    }
    return to_call_payload(payload)


def forwarded_arguments(values: Mapping[str, CallValue]) -> dict[str, CallValue]:
    forwarded: dict[str, CallValue] = {}
    for name in ("api_base", "api_key", "custom_llm_provider", "input", "model"):
        value = values.get(name)
        if value is not None:
            forwarded[name] = value
    return forwarded
