import json
from collections.abc import Callable
from typing import TypeAlias, cast

import litellm
from litellm.llms.openai.chat.gpt_5_transformation import is_gpt_reasoning_series_name
from litellm.llms.openai.chat.gpt_transformation import OpenAIGPTConfig
from litellm.llms.openai.chat.o_series_transformation import OpenAIOSeriesConfig
from litellm.llms.openai.common_utils import OpenAIError
from litellm.llms.openai.openai import OpenAIConfig
from litellm.utils import (  # pyright: ignore[reportPrivateUsage]
    _add_path_to_api_base,  # pyright: ignore[reportPrivateUsage]
    add_openai_metadata,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]
def obj(value: Json) -> Object:
    if not isinstance(value, dict):
        raise TypeError("expected object")
    return value
def text(fields: Object, name: str) -> str:
    value = fields.get(name)
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value
def params(fields: Object) -> Json:
    selected = OpenAIConfig() if fields.get("family", "openai") == "openai" else OpenAIGPTConfig()
    return cast("Json", selected.get_supported_openai_params(model=text(fields, "model")))  # pyright: ignore[reportUnknownMemberType]
def o_series(fields: Object) -> Json:
    return OpenAIOSeriesConfig().is_model_o_series_model(model=text(fields, "model"))
def gpt5(fields: Object) -> Json:
    return is_gpt_reasoning_series_name(text(fields, "model"))
def audio(fields: Object) -> Json:
    model = text(fields, "model")
    return model in cast("set[object]", litellm.open_ai_chat_completion_models) and "audio" in model
def auth(fields: Object) -> Json:
    key = fields.get("api_key") if isinstance(fields.get("api_key"), str) else None
    return cast("Json", OpenAIConfig().validate_environment(  # pyright: ignore[reportUnknownMemberType]
        headers=cast("dict[str, str]", fields.get("headers", {})), model=text(fields, "model"),
        messages=[], optional_params={}, litellm_params={}, api_key=cast("str | None", key),
    ))
def projection(fields: Object) -> Json:
    raw = text(fields, "raw_text")
    decoded = fields.get("decoded")
    if decoded is None:
        json.loads(raw)
    return {"post_call": raw, "response_headers": fields.get("raw_headers", {}), "projected": decoded}  # noqa: E501
def error(fields: Object) -> Json:
    status = fields.get("status_code", 500)
    if not isinstance(status, int):
        raise TypeError("status_code must be an integer")
    failure = OpenAIError(status_code=status, message=text(fields, "message"))
    return {"status_code": failure.status_code, "message": failure.message, "headers": None,
            "request": {"method": failure.request.method, "url": str(failure.request.url)},
            "response": {"status_code": failure.response.status_code,
                         "request": {"method": failure.response.request.method, "url": str(failure.response.request.url)}},  # noqa: E501
            "body": None}

def metadata(fields: Object) -> Json:
    value = fields.get("metadata")
    return cast("Json", add_openai_metadata(None if value is None else obj(value)))
def request(fields: Object) -> Json:
    return cast("Json", OpenAIConfig().transform_request(  # pyright: ignore[reportUnknownMemberType]
        model=text(fields, "model"), messages=cast("list[object]", fields.get("messages", [])),  # pyright: ignore[reportArgumentType]
        optional_params=obj(fields.get("optional", {})), litellm_params={}, headers={},
    ))
def mapped(fields: Object) -> Json:
    body = obj(fields.get("base_parameter_allowlist", fields))
    return cast("Json", OpenAIConfig().map_openai_params(  # pyright: ignore[reportUnknownMemberType]
        obj(body.get("requested", {})), obj(body.get("existing", {})), text(body, "model"), drop_params=False,  # noqa: E501
    ))

def overlap(fields: Object) -> Json:
    return _add_path_to_api_base(text(fields, "api_base"), text(fields, "ending_path"))  # pyright: ignore[reportPrivateUsage]

def azure_gpt5(fields: Object) -> Json:
    model = text(fields, "model")
    return is_gpt_reasoning_series_name(model) or "gpt5_series" in model

HANDLERS: dict[str, Callable[[Object], Json]] = {
    "o_series_routing": o_series, "gpt5_routing": gpt5, "audio_routing": audio,
    "catalog_user_only": params, "old_gpt_response_format_drop": params, "unknown_reasoning_effort": params,  # noqa: E501
    "base_parameter_allowlist": mapped, "base_auth_override": auth,
    "base_projection_forwarding": projection, "base_request_overlay": request,
    "openai_error_defaults": error, "metadata_filter_limit": metadata,
    "overlapping_api_base_path": overlap, "azure_gpt5_route_predicate": azure_gpt5,
}

def run(argument: Json) -> Json:
    fields = obj(argument)
    if len(fields) != 1:
        raise ValueError("expected one openai_base_config variant")
    name, value = next(iter(fields.items()))
    handler = HANDLERS.get(name)
    if handler is None:
        raise ValueError(f"unknown openai_base_config case: {name}")
    return handler(obj(value))
