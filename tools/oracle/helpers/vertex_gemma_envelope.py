from typing import TypeAlias, cast

import litellm
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.vertex_ai.vertex_gemma_models.transformation import VertexGemmaConfig
from litellm.types.llms.openai import AllMessageValues

# an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
litellm.suppress_debug_info = True

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def object_of(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def list_of(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def string_of(fields: dict[str, Json], name: str) -> str:
    value = fields.get(name)
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def transform_section(config: VertexGemmaConfig, section: Json) -> Json:
    fields = object_of(section, "transform case")
    messages = cast(list[AllMessageValues], list_of(fields.get("messages"), "messages"))
    optional_params = dict(object_of(fields.get("optional_params", {}), "optional_params"))
    litellm_params = dict(object_of(fields.get("litellm_params", {}), "litellm_params"))
    request = config.transform_request(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        model=string_of(fields, "model"),
        messages=messages,
        optional_params=optional_params,
        litellm_params=litellm_params,
        headers={},
    )
    return cast(Json, request)


def outcome_section(config: VertexGemmaConfig, section: Json) -> Json:
    response = object_of(section, "response")
    try:
        value = config._unwrap_predictions_response(response)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAny, reportUnknownVariableType]
    except BaseLLMException as error:
        return {"error": {"status": error.status_code}, "predictions": None}
    return {"error": None, "predictions": cast(Json, value)}


def fake_stream_section(config: VertexGemmaConfig, section: Json) -> Json:
    results: list[Json] = []
    for item in list_of(object_of(section, "fake stream case").get("cases"), "cases"):
        query = object_of(item, "fake stream query")
        model = query.get("model")
        stream = query.get("stream")
        provider = query.get("provider")
        results.append(
            {
                "model": model,
                "stream": stream,
                "provider": provider,
                "faked": config.should_fake_stream(
                    model=model if isinstance(model, str) else None,
                    stream=stream if isinstance(stream, bool) else None,
                    custom_llm_provider=provider if isinstance(provider, str) else None,
                ),
            }
        )
    return {"results": results}


def run(input_value: dict[str, Json]) -> Json:
    config = VertexGemmaConfig()
    return {
        "wrapper_keys": transform_section(config, input_value.get("wrapper_keys")),
        "context_management_removed": transform_section(
            config, input_value.get("context_management_removed")
        ),
        "missing_predictions": outcome_section(config, input_value.get("missing_predictions")),
        "embedded_429": outcome_section(config, input_value.get("embedded_429")),
        "invalid_error_code": outcome_section(config, input_value.get("invalid_error_code")),
        "coercible_error_code": outcome_section(config, input_value.get("coercible_error_code")),
        "fake_stream_flag": fake_stream_section(config, input_value.get("fake_stream_flag")),
    }
