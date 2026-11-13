import os
from collections.abc import Callable
from typing import Literal, TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm._logging import verbose_logger
from litellm.llms.vertex_ai import common_utils
from litellm.llms.vertex_ai.common_utils import (
    VertexAILyriaModelInfo,
    _convert_vertex_datetime_to_openai_datetime,  # pyright: ignore[reportPrivateUsage]
    _vertex_ai_lyria_model_key,  # pyright: ignore[reportPrivateUsage]
    get_supports_response_schema,
    get_supports_system_message,
    get_vertex_ai_lyria_generation_cost,
    get_vertex_ai_lyria_model_info,
    get_vertex_ai_model_route,  # pyright: ignore[reportUnknownVariableType]
    is_vertex_self_deployed_openai_compatible_endpoint,
    pop_vertex_request_labels,  # pyright: ignore[reportUnknownVariableType]
    redact_vertex_ai_metadata_from_litellm_params,  # pyright: ignore[reportUnknownVariableType]
    redact_vertex_ai_metadata_from_logged_object,
    supports_response_json_schema,
    vertex_request_labels_from_litellm_params,  # pyright: ignore[reportUnknownVariableType]
)
from litellm.llms.vertex_ai.vertex_ai_partner_models.main import VertexAIPartnerModels

# an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
litellm.suppress_debug_info = True

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Params: TypeAlias = dict[str, Json]
Provider: TypeAlias = Literal["vertex_ai", "vertex_ai_beta", "gemini"]
LookupOutcome: TypeAlias = Literal["true", "false", "error"]
Handler: TypeAlias = Callable[[Params], Json]


def object_of(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def list_of(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def string_of(params: Params, name: str) -> str:
    value = params.get(name)
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def optional_string(params: Params, name: str) -> str | None:
    value = params.get(name)
    return value if isinstance(value, str) else None


def provider_of(fields: Params) -> Provider:
    value = string_of(fields, "provider")
    if value not in ("vertex_ai", "vertex_ai_beta", "gemini"):
        raise ValueError(f"unknown provider {value}")
    return value


def lookup_outcome(fields: Params, name: str) -> LookupOutcome:
    value = string_of(fields, name)
    if value not in ("true", "false", "error"):
        raise ValueError(f"{name} must be true, false or error")
    return value


def route_operation(params: Params) -> Json:
    routes: list[Json] = []
    for item in list_of(params["cases"], "cases"):
        fields = object_of(item, "route case")
        route_params: Params = {}
        base_model = optional_string(fields, "base_model")
        if base_model is not None:
            route_params["base_model"] = base_model
        api_base = optional_string(fields, "api_base")
        if api_base is not None:
            route_params["api_base"] = api_base
        route = get_vertex_ai_model_route(string_of(fields, "model"), route_params)
        routes.append(route.value)
    return {"routes": routes}


def partner_operation(params: Params) -> Json:
    models: list[Json] = []
    for item in list_of(params["models"], "models"):
        model = cast(str, item)
        models.append(
            {
                "model": model,
                "partner": VertexAIPartnerModels.is_vertex_partner_model(model),
                "openai_handler": VertexAIPartnerModels.should_use_openai_handler(model),
                "self_deployed": is_vertex_self_deployed_openai_compatible_endpoint(model),
            }
        )
    return {"models": models}


class WarningRecorder:
    def __init__(self) -> None:
        self.count = 0

    def __call__(self, *_args: object, **_kwargs: object) -> None:
        self.count += 1


class FixtureLookup:
    def __init__(self, outcome: LookupOutcome, provider: str) -> None:
        self.outcome = outcome
        self.provider = provider

    def __call__(
        self,
        model: str,  # noqa: ARG002 - LiteLLM's keyword name
        custom_llm_provider: str | None,
    ) -> bool:
        if custom_llm_provider != self.provider:
            raise RuntimeError(
                f"expected provider {self.provider}, got {custom_llm_provider}"
            )
        if self.outcome == "error":
            raise RuntimeError("capability lookup failed")
        return self.outcome == "true"


def capability_operation(params: Params) -> Json:
    cases: list[Json] = []
    for item in list_of(params["cases"], "cases"):
        fields = object_of(item, "capability case")
        model = string_of(fields, "model")
        provider = provider_of(fields)
        expected_provider = "gemini" if provider == "gemini" else "vertex_ai"
        recorder = WarningRecorder()
        system_lookup = FixtureLookup(lookup_outcome(fields, "system"), expected_provider)
        schema_lookup = FixtureLookup(lookup_outcome(fields, "schema"), expected_provider)
        with (
            patch.object(common_utils, "supports_system_messages", system_lookup),
            patch.object(verbose_logger, "warning", recorder),
        ):
            system = get_supports_system_message(model, provider)
        schema: Json
        with patch.object(common_utils, "supports_response_schema", schema_lookup):
            try:
                schema = get_supports_response_schema(model, provider)
            except Exception:  # noqa: BLE001 - any lookup failure is an error outcome
                schema = {"error": True}
        cases.append(
            {
                "system": system,
                "system_warned": recorder.count > 0,
                "schema": schema,
                "json_schema": supports_response_json_schema(model),
            }
        )
    return {"cases": cases}


def lyria_info_value(info: VertexAILyriaModelInfo | None) -> Json:
    if info is None:
        return None
    formats = cast(Json, list(info["supported_audio_formats"]))
    value: dict[str, Json] = {
        "audio_api": info["vertex_ai_audio_api"],
        "formats": formats,
        "cost": info.get("output_cost_per_image"),
    }
    return value


def lyria_operation(params: Params) -> Json:
    cases: list[Json] = []
    for item in list_of(params["cases"], "cases"):
        fields = object_of(item, "lyria case")
        model = string_of(fields, "model")
        overlay: dict[str, Json] = {}
        if "runtime" in fields:
            key = _vertex_ai_lyria_model_key(model)  # pyright: ignore[reportPrivateUsage]
            overlay[key] = fields["runtime"]
        with patch.object(litellm, "model_cost", overlay):
            info = get_vertex_ai_lyria_model_info(model)
            cost = get_vertex_ai_lyria_generation_cost(model)
        cases.append({"info": lyria_info_value(info), "cost": cost})
    return {"cases": cases}


def labels_operation(params: Params) -> Json:
    cases: list[Json] = []
    for item in list_of(params["cases"], "cases"):
        fields = object_of(item, "label case")
        optional_value = fields.get("optional_params")
        optional_params = (
            dict(object_of(optional_value, "optional_params"))
            if optional_value is not None
            else None
        )
        litellm_value = fields.get("litellm_params")
        litellm_params = (
            object_of(litellm_value, "litellm_params")
            if litellm_value is not None
            else None
        )
        labels = pop_vertex_request_labels(optional_params, litellm_params)
        from_params = vertex_request_labels_from_litellm_params(litellm_params)
        cases.append(
            {
                "labels": cast(Json, labels),
                "from_params": cast(Json, from_params),
                "optional_params": optional_params,
            }
        )
    return {"cases": cases}


def metadata_operation(params: Params) -> Json:
    logged: list[Json] = []
    for item in list_of(params["logged_objects"], "logged_objects"):
        redact_vertex_ai_metadata_from_logged_object(item)
        logged.append(item)
    details: list[Json] = []
    for item in list_of(params["call_details"], "call_details"):
        redact_vertex_ai_metadata_from_litellm_params(
            object_of(item, "call detail")
        )
        details.append(item)
    return {"logged_objects": logged, "call_details": details}


def timestamp_operation(params: Params) -> Json:
    zone = string_of(params, "zone")
    if os.environ.get("TZ") != zone:
        raise ValueError(f"the case TZ must match the zone {zone}")
    timestamps: list[Json] = []
    for item in list_of(params["values"], "values"):
        text = cast(str, item)
        try:
            epoch = _convert_vertex_datetime_to_openai_datetime(text)  # pyright: ignore[reportPrivateUsage]
        except ValueError:
            timestamps.append({"error": True})
        else:
            timestamps.append({"epoch": epoch})
    return {"timestamps": timestamps}


HANDLERS: dict[str, Handler] = {
    "route": route_operation,
    "partner": partner_operation,
    "capabilities": capability_operation,
    "lyria": lyria_operation,
    "labels": labels_operation,
    "metadata": metadata_operation,
    "timestamp": timestamp_operation,
}


def run(input_value: Params) -> Json:
    if len(input_value) != 1:
        raise ValueError("vertex_model_policy takes exactly one operation")
    operation, payload = next(iter(input_value.items()))
    handler = HANDLERS.get(operation)
    if handler is None:
        raise ValueError(f"unknown operation: {operation}")
    return handler(object_of(payload, operation))
