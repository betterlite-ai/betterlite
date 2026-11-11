import os
import sys

os.environ["LITELLM_RUST"] = "false"

from collections.abc import Callable
from contextlib import ExitStack, redirect_stdout
from typing import TYPE_CHECKING, Protocol, TypeAlias, cast
from unittest import mock

import litellm
from litellm.litellm_core_utils.streaming_chunk_builder_utils import ChunkProcessor
from litellm.llms.base_llm.chat.transformation import BaseConfig
from litellm.types.utils import ModelResponse, ModelResponseStream, StreamingChoices, Usage

# get_llm_provider prints a provider list to stdout before raising on an unknown model
litellm.suppress_debug_info = True

if TYPE_CHECKING:
    from litellm.litellm_core_utils.litellm_logging import Logging

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

CHUNK_KEYS = ("id", "object", "created", "model", "system_fingerprint", "service_tier", "choices")
REAL_APPLY: Callable[..., None] = BaseConfig.apply_assembled_streaming_response_metadata


class StreamProcessor(Protocol):
    chunks: list[object]
    first_chunk: object

    def build_base_response(self, chunks: list[object]) -> ModelResponse: ...

    def update_model_response_with_hidden_params(
        self, model_response: ModelResponse, chunk: object | None = None
    ) -> ModelResponse: ...


def attribute(value: object, name: str) -> Json:
    return cast(Json, getattr(value, name, None))  # pyright: ignore[reportAny]


def hidden_value(chunk: object) -> Json:
    if isinstance(chunk, dict):
        raw: object = cast(dict[str, Json], chunk).get("_hidden_params")
    else:
        raw = getattr(chunk, "_hidden_params", None)  # pyright: ignore[reportAny]
    if isinstance(raw, dict):
        return cast(Json, raw)
    return {}


def chunk_view(chunk: object) -> Json:
    if isinstance(chunk, dict):
        fields = cast(dict[str, Json], chunk)
        return {
            "id": fields.get("id"),
            "created": fields.get("created"),
            "model": fields.get("model"),
            "hidden_params": hidden_value(fields),
        }
    return {
        "id": attribute(chunk, "id"),
        "created": attribute(chunk, "created"),
        "model": attribute(chunk, "model"),
        "hidden_params": hidden_value(chunk),
    }


def build_chunk(spec: dict[str, Json]) -> object:
    hidden = spec.get("_hidden_params", {})
    if cast(str, spec.get("form", "dict")) == "model_response_stream":
        chunk = ModelResponseStream(
            id=cast(str | None, spec.get("id")),
            created=cast(int | None, spec.get("created")),
            model=cast(str | None, spec.get("model")),
            system_fingerprint=cast(str | None, spec.get("system_fingerprint")),
            service_tier=cast(str | None, spec.get("service_tier")),
            choices=cast(list[StreamingChoices], spec.get("choices", [])),
        )
        chunk._hidden_params = cast(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAttributeAccessIssue]
            object, hidden
        )
        return chunk
    fields: dict[str, Json] = {key: spec[key] for key in CHUNK_KEYS if key in spec}
    if "_hidden_params" in spec:
        fields["_hidden_params"] = hidden
    return fields


def build_chunks(specs: list[dict[str, Json]]) -> list[object]:
    return [build_chunk(spec) for spec in specs]


def fixture_response(spec: dict[str, Json]) -> ModelResponse:
    response = ModelResponse(
        model=cast(str, spec.get("model", "")),
        id="fixture",
        created=1,
        choices=[],
        usage={"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0},
    )
    response._hidden_params = cast(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportAttributeAccessIssue]
        dict[str, object], spec.get("hidden_params", {})
    )
    return response


def response_view(response: ModelResponse) -> Json:
    usage = cast(Usage | None, getattr(response, "usage", None))  # pyright: ignore[reportAny]
    return {
        "id": response.id,
        "object": response.object,
        "created": response.created,
        "model": response.model,
        "system_fingerprint": response.system_fingerprint,
        "service_tier": attribute(response, "service_tier"),
        "choices": [
            {
                "index": choice.index,
                "message": {"role": choice.message.role, "content": choice.message.content},
                "finish_reason": choice.finish_reason,
                "provider_specific_fields": attribute(choice, "provider_specific_fields"),
            }
            for choice in response.choices
        ],
        "usage": {
            "prompt_tokens": 0 if usage is None else usage.prompt_tokens,
            "completion_tokens": 0 if usage is None else usage.completion_tokens,
            "total_tokens": 0 if usage is None else usage.total_tokens,
        },
        "hidden_params": hidden_value(response),
    }


def stream_processor(chunks: list[object]) -> StreamProcessor:
    return cast(StreamProcessor, ChunkProcessor(chunks))


def sort_case(arguments: dict[str, Json]) -> Json:
    chunks = build_chunks(cast(list[dict[str, Json]], arguments.get("chunks", [])))
    if not chunks:
        return {"ok": False, "error_type": "empty_chunks"}
    processor = stream_processor(chunks)
    return {
        "chunks": [chunk_view(chunk) for chunk in processor.chunks],
        "first_chunk": chunk_view(processor.first_chunk),
    }


def response_case(arguments: dict[str, Json]) -> Json:
    chunks = build_chunks(cast(list[dict[str, Json]], arguments.get("chunks", [])))
    if not chunks:
        return {"ok": False, "error_type": "empty_chunks"}
    processor = stream_processor(chunks)
    try:
        # stream_chunk_builder passes ChunkProcessor.chunks, so the sorted order reaches the builder
        response = processor.build_base_response(processor.chunks)
    except KeyError:
        return {"ok": False, "error_type": "missing_first_field"}
    return response_view(response)


def update_case(arguments: dict[str, Json]) -> Json:
    spec = cast(dict[str, Json], arguments.get("response", {}))
    response = fixture_response(spec)
    raw_chunk = arguments.get("chunk")
    chunk = None if raw_chunk is None else build_chunk(cast(dict[str, Json], raw_chunk))
    processor = stream_processor([{"id": "fixture"}])
    processor.update_model_response_with_hidden_params(response, chunk)
    return {"hidden_params": hidden_value(response)}


class FakeLogging:
    def __init__(self, details: dict[str, Json]) -> None:
        self.model_call_details = details


def record_projection(calls: list[Json]) -> Callable[..., None]:
    def record(config: object, response: ModelResponse, chunks: list[object]) -> None:
        calls.append(
            {"config": type(config).__name__, "model": response.model, "chunks": len(chunks)}
        )
        REAL_APPLY(config, response=response, chunks=chunks)

    return record


def failing_projection(calls: list[Json]) -> Callable[..., None]:
    def fail(config: object, response: ModelResponse, chunks: list[object]) -> None:
        calls.append(
            {"config": type(config).__name__, "model": response.model, "chunks": len(chunks)}
        )
        raise RuntimeError("fixture projection failure")

    return fail


def select_nothing(*_args: object, **_kwargs: object) -> None:
    pass


def select_failing(*_args: object, **_kwargs: object) -> None:
    raise RuntimeError("fixture selector failure")


def logging_details(spec: dict[str, Json]) -> "Logging | None":
    custom = spec.get("custom_llm_provider")
    if custom is None:
        return None
    return cast("Logging", FakeLogging({"custom_llm_provider": cast(Json, custom)}))


def assembled_scenario(spec: dict[str, Json]) -> Json:
    chunks = build_chunks(cast(list[dict[str, Json]], spec.get("chunks", [])))
    response = fixture_response(spec)
    calls: list[Json] = []
    with ExitStack() as stack:
        selector = cast(str, spec.get("selector", "table"))
        if selector == "none":
            stack.enter_context(
                mock.patch(
                    "litellm.utils.ProviderConfigManager.get_provider_chat_config",
                    staticmethod(select_nothing),
                )
            )
        elif selector == "raise":
            stack.enter_context(
                mock.patch(
                    "litellm.utils.ProviderConfigManager.get_provider_chat_config",
                    staticmethod(select_failing),
                )
            )
        if cast(str, spec.get("projection", "record")) == "raise":
            stack.enter_context(
                mock.patch.object(
                    BaseConfig,
                    "apply_assembled_streaming_response_metadata",
                    failing_projection(calls),
                )
            )
        else:
            stack.enter_context(
                mock.patch.object(
                    BaseConfig,
                    "apply_assembled_streaming_response_metadata",
                    record_projection(calls),
                )
            )
        ChunkProcessor.apply_provider_assembled_streaming_metadata(
            response=response,
            chunks=chunks,
            logging_obj=logging_details(spec),
        )
    return {"response": response_view(response), "calls": calls}


def assembled_case(arguments: dict[str, Json]) -> Json:
    scenarios = cast(list[dict[str, Json]], arguments.get("cases", []))
    return [assembled_scenario(spec) for spec in scenarios]


def dispatch(input_value: dict[str, Json]) -> Json:
    function = input_value["function"]
    if function == "batch":
        return [dispatch(case) for case in cast(list[dict[str, Json]], input_value["cases"])]
    if function == "conditional_sort_original_first":
        return sort_case(input_value)
    if function == "base_metadata":
        return response_case(input_value)
    if function == "finish_and_hidden":
        return response_case(input_value)
    if function == "update_hidden_params":
        return update_case(input_value)
    if function == "assembled_provider_metadata":
        return assembled_case(input_value)
    raise ValueError(f"unknown stream_base_assembly function: {function}")


def run(input_value: dict[str, Json]) -> Json:
    # any diagnostic the pinned code prints must not land in the JSON protocol on stdout
    with redirect_stdout(sys.stderr):
        return dispatch(input_value)
