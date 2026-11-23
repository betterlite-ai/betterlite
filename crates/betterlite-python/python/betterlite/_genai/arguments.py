import json
from collections.abc import Mapping
from typing import Final, TypedDict

from pydantic import TypeAdapter

from betterlite._endpoints.payload import CallValue
from betterlite._endpoints.proxy import proxy_headers
from betterlite._endpoints.settings import read_settings

LOGGING_OBJECT: Final = "litellm_logging_obj"
ASYNC_FLAGS: Final[tuple[str, ...]] = ("agenerate_content", "agenerate_content_stream")
FALLBACK_SETTINGS: Final[tuple[str, ...]] = ("api_base", "vertex_project", "vertex_location")
SESSION_SETTINGS: Final[tuple[str, ...]] = ("api_key",)


class SessionSettings(TypedDict):
    api_key: str | None


SESSION: Final[TypeAdapter[SessionSettings]] = TypeAdapter(SessionSettings)


# litellm.api_key is the studio key LiteLLM reads after GOOGLE_API_KEY and GEMINI_API_KEY, so it travels in the session, not in the request
def session_settings() -> str:
    settings = SESSION.validate_python(read_settings(SESSION_SETTINGS))
    api_key = settings["api_key"]
    if api_key is None:
        return "{}"
    return json.dumps({"api_key": api_key})


# LiteLLM's module settings fill a name the call left absent; an explicit value, an empty string included, stays as given
def fill_settings(forwarded: dict[str, CallValue]) -> None:
    settings = read_settings(FALLBACK_SETTINGS)
    for name in FALLBACK_SETTINGS:
        if forwarded.get(name) is None and settings[name] is not None:
            forwarded[name] = settings[name]


# the call's own kwargs minus LiteLLM's async markers and its logging object, which the Rust call cannot take
def forwarded_kwargs(kwargs: Mapping[str, CallValue]) -> dict[str, CallValue]:
    forwarded: dict[str, CallValue] = {
        name: value
        for name, value in kwargs.items()
        if name not in ASYNC_FLAGS and name != LOGGING_OBJECT
    }
    fill_settings(forwarded)
    return forwarded


# proxy auth headers ride every family's request; the caller's own headers go on first, as LiteLLM's completion path orders them
def merged_headers(extra_headers: Mapping[str, str] | None) -> dict[str, str] | None:
    headers: dict[str, str] = {} if extra_headers is None else dict(extra_headers)
    headers.update(proxy_headers())
    return headers or None


# the payload GenaiCall reads: LiteLLM's named arguments on the call, everything else in kwargs for the Rust setup
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
        "kwargs": forwarded,
    }
    return values
