import json
from collections.abc import Callable
from typing import TypeAlias, cast
from unittest.mock import patch

import httpx
import litellm
import litellm.llms.openai.openai as openai_module
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.azure.chat.gpt_5_transformation import AzureOpenAIGPT5Config
from litellm.llms.openai.chat.gpt_5_transformation import is_gpt_reasoning_series_name
from litellm.llms.openai.chat.o_series_transformation import OpenAIOSeriesConfig
from litellm.llms.openai.common_utils import OpenAIError
from litellm.llms.openai.openai import OpenAIConfig
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import ModelResponse
from litellm.utils import _add_path_to_api_base, add_openai_metadata  # pyright: ignore[reportPrivateUsage]

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
    selected = (
        OpenAIConfig() if fields.get("family", "openai") == "openai" else litellm.openAIGPTConfig
    )
    result: list[object] = cast(
        list[object],
        selected.get_supported_openai_params(  # pyright: ignore[reportUnknownMemberType]
            model=text(fields, "model")
        ),
    )
    return cast(Json, result)

def o_series(fields: Object) -> Json:
    return OpenAIOSeriesConfig().is_model_o_series_model(model=text(fields, "model"))

def gpt5(fields: Object) -> Json:
    return is_gpt_reasoning_series_name(text(fields, "model"))

def audio(fields: Object) -> Json:
    return litellm.openAIGPTAudioConfig.is_model_gpt_audio_model(model=text(fields, "model"))

def auth(fields: Object) -> Json:
    key = fields.get("api_key")
    api_key = key if isinstance(key, str) else None
    headers = fields.get("headers", {})
    result: dict[str, object] = cast(
        dict[str, object],
        OpenAIConfig().validate_environment(  # pyright: ignore[reportUnknownMemberType]
            headers=cast(dict[str, str], headers), model=text(fields, "model"), messages=[],
            optional_params={}, litellm_params={}, api_key=api_key),
    )
    return cast(Json, result)

def projection(fields: Object) -> Json:
    events: list[Json] = []
    raw = text(fields, "raw_text")
    raw_headers = cast(dict[str, str], fields.get("raw_headers", {}))

    def recorder(**kwargs: object) -> ModelResponse:
        response_object = kwargs.get("response_object")
        hidden_params = kwargs.get("hidden_params")
        projected_headers = kwargs.get("_response_headers")
        if not isinstance(hidden_params, dict) or "headers" not in hidden_params:
            raise TypeError("missing hidden response headers")
        if not isinstance(projected_headers, dict):
            raise TypeError("missing response headers")
        hidden = cast(dict[str, object], hidden_params)
        hidden_headers = dict(cast(httpx.Headers, hidden["headers"]))
        events.append(cast(Json, [response_object, hidden_headers, projected_headers]))
        return ModelResponse()

    class Recording:
        def __init__(self) -> None:
            self.model_call_details: dict[str, object] = {}

        def post_call(self, original_response: str) -> None:
            events.append(["post_call", original_response])

    logging_obj = Recording()
    result: Object = {"events": events}
    with patch.object(openai_module, "convert_to_model_response_object", side_effect=recorder):
        try:
            OpenAIConfig().transform_response(  # pyright: ignore[reportUnknownMemberType]
                model=text(fields, "model") if "model" in fields else "gpt-4o",
                raw_response=httpx.Response(200, text=raw, headers=raw_headers),
                model_response=ModelResponse(), logging_obj=cast(Logging, logging_obj),
                request_data={}, messages=cast(list[AllMessageValues], []), optional_params={},
                litellm_params={}, encoding=None)
        except json.JSONDecodeError as error:
            result["error"] = type(error).__name__
    logged_headers = dict(cast(httpx.Headers, logging_obj.model_call_details["response_headers"]))
    cast(list[Json], events[0]).append(cast(Json, logged_headers))
    return result

def error(fields: Object) -> Json:
    status = fields.get("status_code")
    if not isinstance(status, int):
        raise TypeError("status_code must be an integer")
    headers = fields.get("headers")
    body = fields.get("body")
    failure = OpenAIError(
        status_code=status, message=text(fields, "message"),
        headers=cast(dict[str, str] | None, headers) if isinstance(headers, dict) else None,
        body=cast(dict[str, Json] | None, body) if isinstance(body, dict) else None)
    request_value = {"method": failure.request.method, "url": str(failure.request.url)}
    response_request = {
        "method": failure.response.request.method,
        "url": str(failure.response.request.url),
    }
    return cast(Json, {
        "status_code": failure.status_code, "message": failure.message,
        "headers": failure.headers, "request": request_value,
        "response": {"status_code": failure.response.status_code, "request": response_request},
        "body": failure.body,
    })

def metadata(fields: Object) -> Json:
    value = fields.get("metadata")
    return cast(Json, add_openai_metadata(None if value is None else obj(value)))

def request(fields: Object) -> Json:
    optional = fields.get("optional", {})
    messages = fields.get("messages", [])
    result: dict[str, object] = cast(
        dict[str, object],
        OpenAIConfig().transform_request(  # pyright: ignore[reportUnknownMemberType]
            model=text(fields, "model"), messages=cast(list[AllMessageValues], messages),
            optional_params=obj(optional), litellm_params={}, headers={}),
    )
    return cast(Json, result)

def mapped(fields: Object) -> Json:
    result: dict[str, object] = cast(
        dict[str, object],
        OpenAIConfig().map_openai_params(  # pyright: ignore[reportUnknownMemberType]
            obj(fields.get("requested", {})), obj(fields.get("existing", {})),
            text(fields, "model"),
            drop_params=False),
    )
    return cast(Json, result)

def overlap(fields: Object) -> Json:
    return _add_path_to_api_base(text(fields, "api_base"), text(fields, "ending_path"))

def azure_gpt5(fields: Object) -> Json:
    return AzureOpenAIGPT5Config.is_model_gpt_5_model(text(fields, "model"))

HANDLERS: dict[str, Callable[[Object], Json]] = {
    "o_series_routing": o_series, "gpt5_routing": gpt5, "audio_routing": audio,
    "catalog_user_only": params, "old_gpt_response_format_drop": params,
    "unknown_reasoning_effort": params, "base_parameter_allowlist": mapped,
    "base_auth_override": auth, "base_projection_forwarding": projection,
    "base_request_overlay": request, "openai_error_defaults": error,
    "metadata_filter_limit": metadata, "overlapping_api_base_path": overlap,
    "azure_gpt5_route_predicate": azure_gpt5,
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
