from datetime import UTC, datetime
from typing import Literal, NotRequired, TypedDict, cast

from litellm.integrations.prometheus import PrometheusLogger
from litellm.types.utils import StandardLoggingPayload
from pydantic import JsonValue


class OverheadCall(TypedDict):
    guardrail_information: JsonValue


class CombinedUsage(TypedDict):
    upstream_total_tokens: int | None


class BucketCall(TypedDict):
    prompt_tokens: int | None
    combined_usage: CombinedUsage | None
    response_has_usage: bool
    metadata_usage: JsonValue
    response_is_object: bool
    response_usage_prompt_tokens: int | None


class DurationCall(TypedDict):
    start_time: float | None
    end_time: float | None


class Input(TypedDict):
    function: Literal["overhead", "input_sequence_length", "safe_duration"]
    overhead_calls: NotRequired[list[OverheadCall]]
    bucket_calls: NotRequired[list[BucketCall]]
    duration_calls: NotRequired[list[DurationCall]]


class UpstreamUsage:
    def __init__(self, total_tokens: int | None) -> None:
        self.total_tokens = total_tokens


class ResponseUsage:
    def __init__(self, usage: dict[str, int | None] | None) -> None:
        self.usage = usage


def as_payload(fields: dict[str, JsonValue]) -> StandardLoggingPayload:
    return cast(StandardLoggingPayload, fields)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L3570


def timestamp(seconds: float | None) -> datetime | None:
    return None if seconds is None else datetime.fromtimestamp(seconds, tz=UTC)


def response_object(call: BucketCall) -> JsonValue | ResponseUsage:
    if not call["response_is_object"]:
        usage = (
            {"prompt_tokens": call["response_usage_prompt_tokens"]}
            if call["response_has_usage"]
            else None
        )
        return ResponseUsage(usage)
    if not call["response_has_usage"]:
        empty: JsonValue = {}
        return empty
    response: JsonValue = {"usage": {"prompt_tokens": call["response_usage_prompt_tokens"]}}
    return response


class ProbeLogger(PrometheusLogger):
    def __init__(self) -> None:
        pass

    def overhead_seconds(self, call: OverheadCall) -> float:
        payload = as_payload({"guardrail_information": call["guardrail_information"]})
        return self._get_guardrail_overhead_seconds(payload)

    def bucket(self, call: BucketCall) -> str:
        payload = as_payload(
            {
                "prompt_tokens": call["prompt_tokens"],
                "metadata": {"usage_object": call["metadata_usage"]},
            }
        )
        combined = call["combined_usage"]
        return self._get_input_sequence_length(
            payload,
            {
                "combined_usage_object": combined,
                "_litellm_upstream_reported_usage": UpstreamUsage(
                    None if combined is None else combined["upstream_total_tokens"]
                ),
            },
            response_object(call),
        )

    def duration(self, call: DurationCall) -> float | None:
        return self._safe_duration_seconds(
            start_time=timestamp(call["start_time"]),
            end_time=timestamp(call["end_time"]),
        )


def run(input_value: Input) -> JsonValue:
    logger = ProbeLogger()
    if input_value["function"] == "overhead":
        overhead: list[JsonValue] = [
            logger.overhead_seconds(call) for call in input_value.get("overhead_calls", [])
        ]
        return {"seconds": overhead}
    if input_value["function"] == "input_sequence_length":
        buckets: list[JsonValue] = [
            logger.bucket(call) for call in input_value.get("bucket_calls", [])
        ]
        return {"buckets": buckets}
    durations: list[JsonValue] = [
        logger.duration(call) for call in input_value.get("duration_calls", [])
    ]
    return {"seconds": durations}
