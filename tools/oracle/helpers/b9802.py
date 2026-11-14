from collections.abc import Mapping
from typing import Final, Literal, TypedDict

from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.common_request_processing import ProxyBaseLLMRequestProcessing
from litellm.types.utils import CostBreakdown
from pydantic import JsonValue, TypeAdapter

JSON: Final = TypeAdapter(JsonValue)


class KeyFields(TypedDict):
    tpm_limit: int | None
    rpm_limit: int | None
    max_budget: float | None
    spend: float
    allowed_model_region: Literal["eu", "us"] | None


class LoggingFields(TypedDict):
    cost_breakdown: CostBreakdown | None
    call_type: str
    litellm_params: dict[str, JsonValue]
    response_timing_metrics: dict[str, float]


class Input(TypedDict):
    key: KeyFields
    call_id: str | None
    model_id: str | None
    cache_key: str | None
    api_base: str | None
    version: str | None
    model_region: str | None
    response_cost: float | str | None
    hidden_params: dict[str, JsonValue]
    # ast-grep-ignore: no-long-names
    fastest_response_batch_completion: bool | None
    request_data: dict[str, JsonValue]
    timeout: float | None
    logging: LoggingFields | None
    read_timing_from_logging: bool
    extra: list[tuple[str, JsonValue]]


class LoggingRecord(Logging):
    cost_breakdown: CostBreakdown | None
    call_type: str
    litellm_params: dict[str, JsonValue]
    response_timing_metrics: Mapping[str, float]

    def __init__(
        self,
        cost_breakdown: CostBreakdown | None,
        call_type: str,
        litellm_params: dict[str, JsonValue],
        timing_metrics: dict[str, float],
    ) -> None:
        self.cost_breakdown = cost_breakdown
        self.call_type = call_type
        self.litellm_params = litellm_params
        self.response_timing_metrics = timing_metrics


def key_auth(fields: KeyFields) -> UserAPIKeyAuth:
    return UserAPIKeyAuth(
        tpm_limit=fields["tpm_limit"],
        rpm_limit=fields["rpm_limit"],
        max_budget=fields["max_budget"],
        spend=fields["spend"],
        allowed_model_region=fields["allowed_model_region"],
    )


def logging_record(fields: LoggingFields | None) -> LoggingRecord | None:
    if fields is None:
        return None
    return LoggingRecord(
        cost_breakdown=fields["cost_breakdown"],
        call_type=fields["call_type"],
        litellm_params=fields["litellm_params"],
        timing_metrics=fields["response_timing_metrics"],
    )


def run(input_value: Input) -> JsonValue:
    return JSON.validate_python(
        ProxyBaseLLMRequestProcessing.get_custom_headers(
            user_api_key_dict=key_auth(input_value["key"]),
            call_id=input_value["call_id"],
            model_id=input_value["model_id"],
            cache_key=input_value["cache_key"],
            api_base=input_value["api_base"],
            version=input_value["version"],
            model_region=input_value["model_region"],
            response_cost=input_value["response_cost"],
            hidden_params=input_value["hidden_params"],
            fastest_response_batch_completion=input_value[
                "fastest_response_batch_completion"
            ],
            request_data=input_value["request_data"],
            timeout=input_value["timeout"],
            litellm_logging_obj=logging_record(input_value["logging"]),
            read_timing_from_logging_obj=input_value["read_timing_from_logging"],
            **dict(input_value["extra"]),
        )
    )
