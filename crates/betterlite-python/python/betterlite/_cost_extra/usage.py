import json
from collections.abc import Mapping, Sequence
from types import MappingProxyType
from typing import Final

from pydantic import JsonValue

from betterlite._cost.engine import native_call
from betterlite._cost.wire import JSON_OBJECT
from betterlite._cost_extra.wire import (
    ENGINE,
    PARTITION,
    JsonObject,
    usage_value,
    usage_wire,
    usages_value,
)
from betterlite.types.utils import LiteLLMRealtimeStreamLoggingObject
from betterlite.types.utils.usage import Usage

REALTIME_SOURCE: Final = "realtime"
RESPONSES_WS_SOURCE: Final = "responses_ws"


def collect_request(source: str, results: Sequence[Mapping[str, JsonValue]]) -> JsonObject:
    return JSON_OBJECT.validate_python(
        {"source": source, "results": [JsonObject(result) for result in results]}
    )


class BaseTokenUsageProcessor:
    @staticmethod
    def combine_usage_objects(usage_objects: list[Usage]) -> Usage:
        request = JSON_OBJECT.validate_python(
            {"usages": [usage_wire(usage) for usage in usage_objects]}
        )
        return usage_value(ENGINE.combine_usage, request)


class RealtimeAPITokenUsageProcessor(BaseTokenUsageProcessor):
    @staticmethod
    def collect_usage_from_realtime_stream_results(  # ast-grep-ignore: no-long-names
        results: Sequence[Mapping[str, JsonValue]],
    ) -> list[Usage]:
        request = collect_request(REALTIME_SOURCE, results)
        return usages_value(ENGINE.collect_usage, request)

    @staticmethod
    def collect_and_combine_usage_from_realtime_stream_results(  # ast-grep-ignore: no-long-names
        results: Sequence[Mapping[str, JsonValue]],
    ) -> Usage:
        collected = RealtimeAPITokenUsageProcessor.collect_usage_from_realtime_stream_results(
            results
        )
        return RealtimeAPITokenUsageProcessor.combine_usage_objects(collected)

    @staticmethod
    def create_logging_realtime_object(
        usage: Usage,
        results: Sequence[Mapping[str, JsonValue]],
    ) -> LiteLLMRealtimeStreamLoggingObject:  # ast-grep-ignore: no-long-names
        return LiteLLMRealtimeStreamLoggingObject(
            usage=usage, results=[JsonObject(result) for result in results]
        )


class ResponsesWebSocketTokenUsageProcessor(  # ast-grep-ignore: no-long-names
    BaseTokenUsageProcessor
):
    @staticmethod
    def collect_usage_from_responses_ws_results(  # ast-grep-ignore: no-long-names
        results: Sequence[Mapping[str, JsonValue]],
    ) -> tuple[Usage, ...]:
        request = collect_request(RESPONSES_WS_SOURCE, results)
        return tuple(usages_value(ENGINE.collect_usage, request))

    @staticmethod
    def partition_results_by_service_tier(  # ast-grep-ignore: no-long-names
        results: Sequence[Mapping[str, JsonValue]],
    ) -> Mapping[str | None, tuple[Mapping[str, JsonValue], ...]]:
        request = JSON_OBJECT.validate_python(
            {"results": [JsonObject(result) for result in results]}
        )
        tiers = PARTITION.validate_json(
            native_call(ENGINE.partition_usage, json.dumps(request))
        )
        return MappingProxyType(
            {service_tier: tuple(events) for service_tier, events in tiers}
        )

    @staticmethod
    def collect_and_combine_usage_from_responses_ws_results(  # ast-grep-ignore: no-long-names
        results: Sequence[Mapping[str, JsonValue]],
    ) -> Usage:
        collected = (
            # ast-grep-ignore: no-long-names
            ResponsesWebSocketTokenUsageProcessor.collect_usage_from_responses_ws_results(
                results
            )
        )
        # ast-grep-ignore: no-long-names
        return ResponsesWebSocketTokenUsageProcessor.combine_usage_objects(
            list(collected)
        )
