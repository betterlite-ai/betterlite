import json
from collections.abc import Mapping, Sequence
from types import MappingProxyType
from typing import Final

from pydantic import JsonValue

from betterlite._cost.engine import native_call
from betterlite._cost_extra.wire import (
    ENGINE,
    PARTITION,
    CollectRequestWire,
    CombineRequestWire,
    JsonObject,
    PartitionRequestWire,
    usage_answer,
    usage_wire,
    usages_answer,
)
from betterlite.types.utils import LiteLLMRealtimeStreamLoggingObject
from betterlite.types.utils.usage import Usage

REALTIME_SOURCE: Final = "realtime"
RESPONSES_WS_SOURCE: Final = "responses_ws"


class BaseTokenUsageProcessor:
    @staticmethod
    def combine_usage_objects(usage_objects: list[Usage]) -> Usage:
        request = CombineRequestWire(
            usages=[usage_wire(usage) for usage in usage_objects]
        )
        return usage_answer(ENGINE.combine_usage, json.dumps(request))


class RealtimeAPITokenUsageProcessor(BaseTokenUsageProcessor):
    @staticmethod
    def collect_usage_from_realtime_stream_results(  # ast-grep-ignore: no-long-names
        results: Sequence[Mapping[str, JsonValue]],
    ) -> list[Usage]:
        request = CollectRequestWire(
            source=REALTIME_SOURCE,
            results=[JsonObject(result) for result in results],
        )
        return usages_answer(ENGINE.collect_usage, json.dumps(request))

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
    ) -> LiteLLMRealtimeStreamLoggingObject:
        return LiteLLMRealtimeStreamLoggingObject(
            usage=usage, results=[JsonObject(result) for result in results]
        )


class ResponsesWebSocketTokenUsageProcessor(BaseTokenUsageProcessor):
    @staticmethod
    def collect_usage_from_responses_ws_results(  # ast-grep-ignore: no-long-names
        results: Sequence[Mapping[str, JsonValue]],
    ) -> tuple[Usage, ...]:
        request = CollectRequestWire(
            source=RESPONSES_WS_SOURCE,
            results=[JsonObject(result) for result in results],
        )
        return tuple(usages_answer(ENGINE.collect_usage, json.dumps(request)))

    @staticmethod
    def partition_results_by_service_tier(  # ast-grep-ignore: no-long-names
        results: Sequence[Mapping[str, JsonValue]],
    ) -> Mapping[str | None, tuple[Mapping[str, JsonValue], ...]]:
        request = PartitionRequestWire(
            results=[JsonObject(result) for result in results]
        )
        answer = PARTITION.validate_json(
            native_call(ENGINE.partition_usage, json.dumps(request))
        )
        return MappingProxyType(
            {
                group["service_tier"]: tuple(group["events"])
                for group in answer["tiers"]
            }
        )

    @staticmethod
    def collect_and_combine_usage_from_responses_ws_results(  # ast-grep-ignore: no-long-names
        results: Sequence[Mapping[str, JsonValue]],
    ) -> Usage:
        collected = (
            ResponsesWebSocketTokenUsageProcessor.collect_usage_from_responses_ws_results(
                results
            )
        )
        return ResponsesWebSocketTokenUsageProcessor.combine_usage_objects(
            list(collected)
        )
