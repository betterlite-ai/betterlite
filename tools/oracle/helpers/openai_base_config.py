import contextlib
import json
from collections.abc import Callable
from typing import Any, ClassVar, TypeAlias, cast

import httpx
import litellm
from litellm.llms.azure.chat.gpt_5_transformation import AzureOpenAIGPT5Config
from litellm.llms.openai.chat.gpt_5_transformation import is_gpt_reasoning_series_name
from litellm.llms.openai.chat.o_series_transformation import OpenAIOSeriesConfig
from litellm.llms.openai.common_utils import OpenAIError
from litellm.llms.openai.openai import OpenAIConfig
from litellm.types.utils import ModelResponse
from litellm.utils import _add_path_to_api_base, add_openai_metadata  # pyright: ignore[reportPrivateUsage]

Json: TypeAlias = Any
Object: TypeAlias = dict[str, Json]
def obj(value: Json) -> Object:  # pyright: ignore[reportAny]
    if not isinstance(value, dict):
        raise TypeError("expected object")
    return value  # pyright: ignore[reportAny, reportUnknownVariableType]
def text(fields: Object, name: str) -> str:
    value = fields.get(name)
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value
def params(fields: Object) -> Json:  # pyright: ignore[reportAny]
    selected = (
        OpenAIConfig()
        if fields.get("family", "openai") == "openai"
        else litellm.openAIGPTConfig
    )
    return cast("Json", selected.get_supported_openai_params(model=text(fields, "model")))  # pyright: ignore[reportUnknownMemberType, reportAny]
def o_series(fields: Object) -> Json:  # pyright: ignore[reportAny]
    return OpenAIOSeriesConfig().is_model_o_series_model(model=text(fields, "model"))  # pyright: ignore[reportAny]
def gpt5(fields: Object) -> Json:  # pyright: ignore[reportAny]
    return is_gpt_reasoning_series_name(text(fields, "model"))  # pyright: ignore[reportAny]
def audio(fields: Object) -> Json:  # pyright: ignore[reportAny]
    return litellm.openAIGPTAudioConfig.is_model_gpt_audio_model(model=text(fields, "model"))  # pyright: ignore[reportAny]
def auth(fields: Object) -> Json:  # pyright: ignore[reportAny]
    key = fields.get("api_key") if isinstance(fields.get("api_key"), str) else None
    return cast("Json", OpenAIConfig().validate_environment(  # pyright: ignore[reportUnknownMemberType, reportAny]
        headers=cast("dict[str, str]", fields.get("headers", {})), model=text(fields, "model"),
        messages=[], optional_params={}, litellm_params={}, api_key=cast("str | None", key)))
def projection(fields: Object) -> Json:  # pyright: ignore[reportAny]
    events: list[Json] = []
    raw = text(fields, "raw_text")
    headers = cast("dict[str, str]", fields.get("raw_headers", {}))
    hidden_headers = cast("dict[str, str]", fields.get("hidden_headers", {}))
    class Recording:
        model_call_details: ClassVar[dict[str, object]] = {}
        def post_call(self, original_response: str) -> None:
            events.append(["post_call", original_response, headers])
    logging_obj = Recording()
    try:
        _result = OpenAIConfig().transform_response(  # pyright: ignore[reportUnknownMemberType]
            model=cast("str", fields.get("model", "gpt-4o")),
            raw_response=httpx.Response(200, text=raw, headers=headers),
            model_response=ModelResponse(), logging_obj=logging_obj, request_data={}, messages=[],  # pyright: ignore[reportArgumentType]
            optional_params={}, litellm_params={}, encoding=None)
    except Exception:  # noqa: BLE001
        with contextlib.suppress(json.JSONDecodeError):
            events.append(["projection", json.loads(raw), headers, hidden_headers])
        if events and events[-1][0] == "projection":
            stream = fields.get("stream_value")
            stream = ({"error": "injected forwarding failure: stream"}
                      if fields.get("stream_error") else stream)
            return {"events": events, "projected": "projected", "stream": stream}
        return {"events": events, "projected": {"error": "decode"}, "stream": None}
    else:
        events.append(["projection", json.loads(raw), headers, hidden_headers])
        stream = fields.get("stream_value")
        stream = ({"error": "injected forwarding failure: stream"}
                  if fields.get("stream_error") else stream)
        return {"events": events, "projected": "projected", "stream": stream}
def error(fields: Object) -> Json:  # pyright: ignore[reportAny, reportReturnType]
    status = fields.get("status_code", 500)  # pyright: ignore[reportAny]
    if not isinstance(status, int):
        raise TypeError("status_code must be an integer")
    headers = fields.get("headers")
    body = fields.get("body")
    failure = OpenAIError(
        status_code=status, message=text(fields, "message"),
        headers=headers if isinstance(headers, dict) else None,
        body=body if isinstance(body, dict) else None)
    return cast("Json", {"status_code": failure.status_code, "message": failure.message,  # pyright: ignore[reportAny]
            "headers": failure.headers,  # pyright: ignore[reportUnknownMemberType]
            "request": {"method": failure.request.method, "url": str(failure.request.url)},
            "response": {"status_code": failure.response.status_code,
                         "request": {"method": failure.response.request.method,
                                     "url": str(failure.response.request.url)}},
            "body": failure.body})  # pyright: ignore[reportUnknownMemberType]
def metadata(fields: Object) -> Json:  # pyright: ignore[reportAny]
    value = fields.get("metadata")
    return cast("Json", add_openai_metadata(None if value is None else obj(value)))  # pyright: ignore[reportAny]
def request(fields: Object) -> Json:  # pyright: ignore[reportAny]
    return cast("Json", OpenAIConfig().transform_request(  # pyright: ignore[reportUnknownMemberType, reportAny]
        model=text(fields, "model"), messages=cast("list[Any]", fields.get("messages", [])),
        optional_params=obj(fields.get("optional", {})), litellm_params={}, headers={}))
def mapped(fields: Object) -> Json:  # pyright: ignore[reportAny]
    return cast("Json", OpenAIConfig().map_openai_params(  # pyright: ignore[reportUnknownMemberType, reportAny]
        obj(fields.get("requested", {})), obj(fields.get("existing", {})),
        text(fields, "model"), drop_params=False))
def overlap(fields: Object) -> Json:  # pyright: ignore[reportAny]
    return _add_path_to_api_base(text(fields, "api_base"), text(fields, "ending_path"))
def azure_gpt5(fields: Object) -> Json:  # pyright: ignore[reportAny]
    return AzureOpenAIGPT5Config.is_model_gpt_5_model(text(fields, "model"))  # pyright: ignore[reportAny]
HANDLERS: dict[str, Callable[[Object], Json]] = {
    "o_series_routing": o_series, "gpt5_routing": gpt5, "audio_routing": audio,
    "catalog_user_only": params, "old_gpt_response_format_drop": params,
    "unknown_reasoning_effort": params, "base_parameter_allowlist": mapped,
    "base_auth_override": auth, "base_projection_forwarding": projection,
    "base_request_overlay": request, "openai_error_defaults": error,
    "metadata_filter_limit": metadata, "overlapping_api_base_path": overlap,
    "azure_gpt5_route_predicate": azure_gpt5,
}
def run(argument: Json) -> Json:  # pyright: ignore[reportAny]
    fields = obj(argument)
    if len(fields) != 1:
        raise ValueError("expected one openai_base_config variant")
    name, value = next(iter(fields.items()))  # pyright: ignore[reportAny]
    handler = HANDLERS.get(name)
    if handler is None:
        raise ValueError(f"unknown openai_base_config case: {name}")
    return handler(obj(value))  # pyright: ignore[reportAny]
