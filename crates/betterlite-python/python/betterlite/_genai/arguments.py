import json
from collections.abc import Mapping
from typing import Final, TypedDict

from pydantic import TypeAdapter

from betterlite._endpoints.payload import LOGGER_FN, REFUSED, CallValue
from betterlite._endpoints.proxy import proxy_headers
from betterlite._endpoints.settings import read_settings

LOGGING_OBJECT: Final = "litellm_logging_obj"
ASYNC_FLAGS: Final[tuple[str, ...]] = ("agenerate_content", "agenerate_content_stream")
TOP_LEVEL: Final[tuple[str, ...]] = (*REFUSED, LOGGER_FN)
FALLBACK_SETTINGS: Final[tuple[str, ...]] = ("vertex_project", "vertex_location")
SESSION_SETTINGS: Final[tuple[str, ...]] = ("api_key",)


class SessionSettings(TypedDict):
    api_key: str | None


SESSION: Final[TypeAdapter[SessionSettings]] = TypeAdapter(SessionSettings)


# litellm.api_key is the studio key LiteLLM reads after GOOGLE_API_KEY and GEMINI_API_KEY
def session_settings() -> str:
    settings = SESSION.validate_python(read_settings(SESSION_SETTINGS))
    api_key = settings["api_key"]
    if api_key is None:
        return "{}"
    payload: dict[str, str] = {"api_key": api_key}
    return json.dumps(payload)


# module settings fill a name the call left absent; an explicit value, an empty one included
def fill_settings(forwarded: dict[str, CallValue]) -> None:
    settings = read_settings(FALLBACK_SETTINGS)
    for name in FALLBACK_SETTINGS:
        value = settings[name]
        if value is not None and forwarded.get(name) is None:
            forwarded[name] = value


# the call's own kwargs minus LiteLLM's async markers and its logging object
def forwarded_kwargs(kwargs: Mapping[str, CallValue]) -> dict[str, CallValue]:
    forwarded: dict[str, CallValue] = {}
    for name, value in kwargs.items():
        if name in ASYNC_FLAGS or name == LOGGING_OBJECT:
            continue
        forwarded[name] = value
    fill_settings(forwarded)
    return forwarded


# proxy auth headers ride every family's request, after the caller's own headers
def merged_headers(extra_headers: Mapping[str, str] | None) -> dict[str, str]:
    headers: dict[str, str] = {} if extra_headers is None else dict(extra_headers)
    headers.update(proxy_headers())
    return headers


# the payload GenaiCall reads: LiteLLM's named arguments, everything else in kwargs
def call_values(
    model: str, contents: CallValue, config: CallValue, tools: CallValue,
    extra_headers: Mapping[str, str] | None, extra_query: CallValue,
    extra_body: CallValue, timeout: CallValue, custom_llm_provider: str | None,
    kwargs: Mapping[str, CallValue],
) -> dict[str, CallValue]:
    forwarded = forwarded_kwargs(kwargs)
    values: dict[str, CallValue] = {
        "model": model,
        "contents": contents,
        "config": config,
        "tools": tools,
        "extra_headers": merged_headers(extra_headers),
        "extra_query": extra_query,
        "extra_body": extra_body,
        "timeout": timeout,
        "custom_llm_provider": custom_llm_provider,
    }
    # to_call_payload refuses or drops these names at the payload's top level, not inside kwargs
    for name in TOP_LEVEL:
        if name in forwarded:
            values[name] = forwarded.pop(name)
    values["kwargs"] = forwarded
    return values
