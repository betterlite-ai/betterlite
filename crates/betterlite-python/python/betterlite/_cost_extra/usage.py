import json
from collections.abc import Mapping, Sequence
from types import MappingProxyType
from typing import Final

from pydantic import JsonValue

from betterlite._cost.wire import JSON_OBJECT
from betterlite._cost_extra.wire import (
    ENGINE,
    PARTITION,
    USAGE,
    USAGES,
    CollectUsageRequestWire,
    CombineUsageRequestWire,
    JsonObject,
    PartitionUsageRequestWire,
    native_answer,
    usage_wire,
)
from betterlite.types.utils import LiteLLMRealtimeStreamLoggingObject
from betterlite.types.utils.usage import PRIVATE_CACHE_PARAMS, Usage

REALTIME_SOURCE: Final = "realtime"
RESPONSES_WS_SOURCE: Final = "responses_ws"


def collect_request(
    source: str, results: Sequence[Mapping[str, JsonValue]]
) -> CollectUsageRequestWire:
    request: CollectUsageRequestWire = {
        "source": source,
        "results": [dict(result) for result in results],
    }
    return request


class BaseTokenUsageProcessor:
    @staticmethod
    def combine_usage_objects(usage_objects: list[Usage]) -> Usage:
        usages: list[JsonObject] = [usage_wire(usage) for usage in usage_objects]
        request: CombineUsageRequestWire = {"usages": usages}
        wire: JsonObject = native_answer(JSON_OBJECT, ENGINE.combine_usage, json.dumps(request))
        # LiteLLM keeps the cache aliases beside the summed details, so they go back on afterwards.
        aliases = {name: wire.pop(name) for name, _ in PRIVATE_CACHE_PARAMS if name in wire}
        combined: Usage = USAGE.validate_python(wire)
        for name, value in aliases.items():
            combined[name] = value
        return combined


class RealtimeAPITokenUsageProcessor(BaseTokenUsageProcessor):
    @staticmethod
    def collect_usage_from_realtime_stream_results(  # ast-grep-ignore: no-long-names
        results: Sequence[Mapping[str, JsonValue]],
    ) -> list[Usage]:
        request = collect_request(REALTIME_SOURCE, results)
        return native_answer(USAGES, ENGINE.collect_usage, json.dumps(request))

    @staticmethod
    def collect_and_combine_usage_from_realtime_stream_results(  # ast-grep-ignore: no-long-names
        results: Sequence[Mapping[str, JsonValue]],
    ) -> Usage:
        processor = RealtimeAPITokenUsageProcessor
        collected = processor.collect_usage_from_realtime_stream_results(results)
        return processor.combine_usage_objects(collected)

    @staticmethod
    def create_logging_realtime_object(
        usage: Usage,
        results: Sequence[Mapping[str, JsonValue]],
    ) -> LiteLLMRealtimeStreamLoggingObject:  # ast-grep-ignore: no-long-names
        return LiteLLMRealtimeStreamLoggingObject(
            usage=usage, results=[dict(result) for result in results]
        )


class ResponsesWebSocketTokenUsageProcessor(  # ast-grep-ignore: no-long-names
    BaseTokenUsageProcessor
):
    @staticmethod
    def collect_usage_from_responses_ws_results(  # ast-grep-ignore: no-long-names
        results: Sequence[Mapping[str, JsonValue]],
    ) -> tuple[Usage, ...]:
        request = collect_request(RESPONSES_WS_SOURCE, results)
        collected: list[Usage] = native_answer(USAGES, ENGINE.collect_usage, json.dumps(request))
        return tuple(collected)

    @staticmethod
    def partition_results_by_service_tier(  # ast-grep-ignore: no-long-names
        results: Sequence[Mapping[str, JsonValue]],
    ) -> Mapping[str | None, tuple[Mapping[str, JsonValue], ...]]:
        request: PartitionUsageRequestWire = {"results": [dict(result) for result in results]}
        tiers = native_answer(PARTITION, ENGINE.partition_usage, json.dumps(request))
        return MappingProxyType({service_tier: tuple(events) for service_tier, events in tiers})

    @staticmethod
    def collect_and_combine_usage_from_responses_ws_results(  # ast-grep-ignore: no-long-names
        results: Sequence[Mapping[str, JsonValue]],
    ) -> Usage:
        # ast-grep-ignore: no-long-names
        processor = ResponsesWebSocketTokenUsageProcessor
        collected = processor.collect_usage_from_responses_ws_results(results)
        return processor.combine_usage_objects(list(collected))
