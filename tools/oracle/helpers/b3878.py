from datetime import UTC, datetime
from typing import Literal, TypeAlias, TypedDict, cast

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


class OverheadInput(TypedDict):
    function: Literal["overhead"]
    overhead_calls: list[OverheadCall]


class BucketInput(TypedDict):
    function: Literal["input_sequence_length"]
    bucket_calls: list[BucketCall]


class DurationInput(TypedDict):
    function: Literal["safe_duration"]
    duration_calls: list[DurationCall]


Input: TypeAlias = OverheadInput | BucketInput | DurationInput


class UpstreamUsage:
    def __init__(self, total_tokens: int | None) -> None:
        self.total_tokens = total_tokens


class ResponseUsage:
    def __init__(self, usage: dict[str, int | None] | None) -> None:
        self.usage = usage


def response_object(call: BucketCall) -> JsonValue | ResponseUsage:
    if not call["response_is_object"]:
        return ResponseUsage(
            {"prompt_tokens": call["response_usage_prompt_tokens"]}
            if call["response_has_usage"]
            else None
        )
    if not call["response_has_usage"]:
        return {}
    return {"usage": {"prompt_tokens": call["response_usage_prompt_tokens"]}}


class ProbeLogger(PrometheusLogger):
    def __init__(self) -> None:
        pass

    def overhead_seconds(self, call: OverheadCall) -> float:
        payload = cast(StandardLoggingPayload, {"guardrail_information": call["guardrail_information"]})  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L3570
        return self._get_guardrail_overhead_seconds(payload)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L3570

    def bucket(self, call: BucketCall) -> str:
        combined = call["combined_usage"]
        return self._get_input_sequence_length(
            cast(StandardLoggingPayload, {"prompt_tokens": call["prompt_tokens"], "metadata": {"usage_object": call["metadata_usage"]}}),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L3570
            {
                "combined_usage_object": combined,
                "_litellm_upstream_reported_usage": UpstreamUsage(
                    None if combined is None else combined["upstream_total_tokens"]
                ),
            },
            response_object(call),
        )

    def duration(self, call: DurationCall) -> float | None:
        start_time = call["start_time"]
        end_time = call["end_time"]
        return self._safe_duration_seconds(
            start_time=None if start_time is None else datetime.fromtimestamp(start_time, tz=UTC),
            end_time=None if end_time is None else datetime.fromtimestamp(end_time, tz=UTC),
        )


def run(input_value: Input) -> JsonValue:
    logger = ProbeLogger()
    match input_value["function"]:
        case "overhead":
            overhead: list[JsonValue] = [
                logger.overhead_seconds(call) for call in input_value["overhead_calls"]
            ]
            return {"seconds": overhead}
        case "input_sequence_length":
            buckets: list[JsonValue] = [
                logger.bucket(call) for call in input_value["bucket_calls"]
            ]
            return {"buckets": buckets}
        case "safe_duration":
            durations: list[JsonValue] = [
                logger.duration(call) for call in input_value["duration_calls"]
            ]
            return {"seconds": durations}
