from collections.abc import Collection, Mapping
from typing import Final, Protocol

from pydantic import JsonValue

import betterlite
from betterlite._native import is_litellm_owned_kwarg
from betterlite.types.videos.main import VideoCreateOptionalRequestParams as CreateParams

EXCLUDED_PARAMS: Final[frozenset[str]] = frozenset(
    {"kwargs", "extra_body", "prompt", "model"}
)
NO_EXCLUSIONS: Final[frozenset[str]] = frozenset()


class VideoProviderConfig(Protocol):
    def map_openai_params(
        self,
        *,
        video_create_optional_params: CreateParams,
        model: str,
        drop_params: bool,
    ) -> dict[str, JsonValue]: ...


# LiteLLM's filter_out_litellm_params: the names LiteLLM owns never reach a provider body
def filter_out_litellm_params(
    kwargs: Mapping[str, JsonValue],
    excluding: Collection[str] = NO_EXCLUSIONS,
) -> dict[str, JsonValue]:
    return {
        key: value
        for key, value in kwargs.items()
        if key not in excluding and not is_litellm_owned_kwarg(key)
    }


# LiteLLM checks isinstance(candidate, dict); a JSON value says the same with class patterns
def json_map(value: JsonValue | None) -> dict[str, JsonValue]:
    match value:
        case dict() as entries:
            return entries
        case bool() | int() | float() | str() | list() | None:
            return {}


def map_video_optional_params(
    model: str,
    provider_config: VideoProviderConfig,
    optional_params: CreateParams,
) -> dict[str, JsonValue]:
    mapped_params = provider_config.map_openai_params(
        video_create_optional_params=optional_params,
        model=model,
        drop_params=betterlite.drop_params,
    )
    if "extra_body" in optional_params:
        extra_body = optional_params["extra_body"]
        if extra_body:
            mapped_params.update(extra_body)
        mapped_params.pop("extra_body", None)
    return mapped_params


def requested_optional_params(
    params: Mapping[str, JsonValue],
) -> dict[str, JsonValue]:
    given = dict(params)
    raw_kwargs = dict(json_map(given.get("kwargs")))
    kwargs_extra_body = raw_kwargs.pop("extra_body", None)
    top_level_extra_body = given.get("extra_body")
    base_params = filter_out_litellm_params({
        key: value
        for key, value in given.items()
        if key not in EXCLUDED_PARAMS and value is not None
    })
    cleaned_kwargs = filter_out_litellm_params({
        key: value for key, value in raw_kwargs.items() if value is not None
    })
    optional_params: dict[str, JsonValue] = {**base_params, **cleaned_kwargs}
    merged_extra_body: dict[str, JsonValue] = {}
    for candidate in (top_level_extra_body, kwargs_extra_body):
        merged_extra_body.update({
            key: value
            for key, value in json_map(candidate).items()
            if value is not None
        })
    if merged_extra_body:
        merged_extra_body = filter_out_litellm_params(merged_extra_body)
        if merged_extra_body:
            optional_params["extra_body"] = merged_extra_body
            optional_params.update(merged_extra_body)
    optional_params.pop("timeout", None)
    return optional_params
