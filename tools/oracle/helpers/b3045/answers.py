from typing import Literal

from litellm.types.llms.openai import ResponseAPIUsage
from litellm.types.rerank import RerankResponse
from litellm.types.utils import EmbeddingResponse, Usage
from pydantic import JsonValue, TypeAdapter

from .inputs import (
    CallbackArguments,
    IoTokenCase,
    ResponseInput,
    ResponsePayload,
    ScopeTargetsCase,
    SuccessCase,
)
from .operations import (
    Operations,
    ReservationAnswer,
    SuccessAnswer,
    TargetsAnswer,
    install_stash,
)


def rerank_response(input_value: ResponseInput) -> RerankResponse:
    metadata: dict[str, JsonValue] = {}
    tokens = input_value.get("rerank_tokens")
    if tokens is not None:
        metadata["tokens"] = {
            "input_tokens": tokens["input_tokens"],
            "output_tokens": tokens["output_tokens"],
        }
    billed_total = input_value.get("billed_units_total_tokens")
    if billed_total is not None:
        metadata["billed_units"] = {"total_tokens": billed_total}
    return RerankResponse.model_validate({"meta": metadata})


def response_api_response(
    usage: dict[str, JsonValue] | None,
) -> ResponseAPIUsage | EmbeddingResponse:
    if usage is None:
        return EmbeddingResponse()
    return TypeAdapter(ResponseAPIUsage).validate_python(usage)


def embedding_response(usage: dict[str, JsonValue] | None) -> EmbeddingResponse:
    if usage is None:
        return EmbeddingResponse()
    return EmbeddingResponse(usage=TypeAdapter(Usage).validate_python(usage))


def response_object(input_value: ResponseInput | None) -> ResponsePayload:
    if input_value is None:
        return EmbeddingResponse()
    kind = input_value.get("kind")
    if kind == "rerank":
        return rerank_response(input_value)
    if kind == "unmodelled":
        return None
    if kind == "response_api":
        return response_api_response(input_value.get("response_api_usage"))
    usage = input_value.get("usage")
    if kind == "mapping" and usage is not None:
        return {"usage": usage}
    return embedding_response(usage)


def base_arguments(
    call_id: str | None,
    combined: dict[str, JsonValue] | None,
    request_user: str | None,
) -> CallbackArguments:
    arguments = CallbackArguments()
    if call_id is not None:
        arguments["litellm_call_id"] = call_id
    if request_user is not None:
        arguments["user"] = request_user
    if combined is not None:
        arguments["combined_usage_object"] = TypeAdapter(Usage).validate_python(combined)
    return arguments


def success_arguments(input_value: SuccessCase) -> CallbackArguments:
    raw_metadata = input_value.get("request_metadata")
    metadata: dict[str, JsonValue] = {} if raw_metadata is None else dict(raw_metadata)
    model_group = input_value.get("model_group")
    if model_group is not None:
        metadata["model_group"] = model_group
    raw_logging = input_value.get("logging")
    logging_metadata: dict[str, JsonValue] = {} if raw_logging is None else raw_logging
    arguments = base_arguments(
        input_value.get("call_id"),
        input_value.get("combined_usage"),
        input_value.get("request_user"),
    )
    arguments["litellm_params"] = {"metadata": metadata}
    arguments["standard_logging_object"] = {"metadata": logging_metadata}
    return arguments


def io_token_answer(input_value: IoTokenCase, limiter: Operations) -> ReservationAnswer:
    stash_input = input_value.get("stash")
    if stash_input is not None:
        install_stash(stash_input)
    arguments = base_arguments(
        input_value.get("call_id"), input_value.get("combined_usage"), None
    )
    response = response_object(input_value.get("response"))
    return Operations.io_token_ops(limiter, arguments, response)


def scope_targets_answer(
    input_value: ScopeTargetsCase, limiter: Operations
) -> TargetsAnswer:
    raw_logging = input_value.get("logging")
    logging_metadata: dict[str, JsonValue] = {} if raw_logging is None else raw_logging
    raw_tags = input_value.get("tags")
    tag_set: set[str] = set() if raw_tags is None else set(raw_tags)
    arguments = base_arguments(None, None, input_value.get("request_user"))
    return Operations.scope_targets(
        limiter, logging_metadata, arguments, input_value.get("model_group"), tag_set
    )


def success_answer(input_value: SuccessCase, limiter: Operations) -> SuccessAnswer:
    stash_input = input_value.get("stash")
    if stash_input is not None:
        install_stash(stash_input)
    configured = input_value.get("rate_limit_type")
    rate_limit_type: Literal["input", "output", "total"] = "total"
    if configured is not None:
        rate_limit_type = configured
    arguments = success_arguments(input_value)
    response = response_object(input_value.get("response"))
    return Operations.success_ops(limiter, arguments, response, rate_limit_type)
