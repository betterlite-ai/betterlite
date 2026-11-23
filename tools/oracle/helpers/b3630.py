from datetime import UTC, datetime
from typing import Literal, TypedDict

from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.proxy.pass_through_endpoints.llm_provider_handlers.anthropic_passthrough_logging_handler import (
    AnthropicPassthroughLoggingHandler as Handler,
)
from litellm.proxy.pass_through_endpoints.llm_provider_handlers.batch_attribution import (
    is_collection_route,
    request_tags_from_metadata,
)
from pydantic import JsonValue


class RouteCall(TypedDict):
    url_route: str
    collection_suffix: str


class TagsCall(TypedDict):
    metadata: dict[str, JsonValue]


class ChunksCall(TypedDict):
    chunks: list[str]


class CostingCall(TypedDict):
    model: str
    details: dict[str, JsonValue]


class SpeedCall(TypedDict):
    request_body: dict[str, JsonValue] | None


class Input(TypedDict):
    routes: list[RouteCall]
    tags: list[TagsCall]
    models: list[ChunksCall]
    ids: list[ChunksCall]
    costing: list[CostingCall]
    speeds: list[SpeedCall]


class Answer(TypedDict):
    routes: list[bool]
    tags: list[list[str] | None]
    models: list[str | None]
    ids: list[str | None]
    costing: list[str]
    speeds: list[str | None]


def tag_answer(call: TagsCall) -> list[str] | None:
    tags = request_tags_from_metadata(call["metadata"])
    if tags is None:
        return None
    return list(tags)


def field_answer(chunks: list[str], field: Literal["model", "id"]) -> str | None:
    decoded = [bytes.fromhex(chunk) for chunk in chunks]
    if field == "model":
        return Handler._extract_model_from_anthropic_chunks(decoded)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_provider_handlers/anthropic_passthrough_logging_handler.py#L317-L322
    return Handler._extract_response_id_from_anthropic_chunks(decoded)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_provider_handlers/anthropic_passthrough_logging_handler.py#L323-L328


def costing_answer(call: CostingCall) -> str:
    log = Logging(
        model="",
        messages=None,
        stream=False,
        call_type="pass_through_endpoint",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="b3630-call",
        function_id="b3630",
    )
    log.model_call_details = call["details"]
    return Handler._resolve_costing_model(call["model"], log)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_provider_handlers/anthropic_passthrough_logging_handler.py#L139-L151


def run(input_value: Input) -> Answer:
    return {
        "routes": [
            is_collection_route(call["url_route"], call["collection_suffix"])
            for call in input_value["routes"]
        ],
        "tags": [tag_answer(call) for call in input_value["tags"]],
        "models": [
            field_answer(call["chunks"], "model") for call in input_value["models"]
        ],
        "ids": [field_answer(call["chunks"], "id") for call in input_value["ids"]],
        "costing": [costing_answer(call) for call in input_value["costing"]],
        "speeds": [
            Handler._cost_relevant_speed(call["request_body"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/pass_through_endpoints/llm_provider_handlers/anthropic_passthrough_logging_handler.py#L119-L130
            for call in input_value["speeds"]
        ],
    }
