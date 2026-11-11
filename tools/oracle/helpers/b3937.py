from typing import Literal, TypeAlias, TypedDict, cast

from litellm.integrations.datadog import datadog_llm_obs
from litellm.types.integrations.datadog_llm_obs import DDLLMObsError, LLMMetrics
from litellm.types.utils import StandardLoggingPayload

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

_LOGGER = object.__new__(datadog_llm_obs.DataDogLLMObsLogger)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/datadog/datadog_llm_obs.py#L440


class SpanKindInput(TypedDict):
    function: Literal["span_kind"]
    call_type: str | None
    parent_id: str | None


class MetricsInput(TypedDict):
    function: Literal["metrics"]
    payload: dict[str, Json]
    tool_output_tokens: float | None


class ErrorInfoInput(TypedDict):
    function: Literal["error_info"]
    payload: dict[str, Json]


Input: TypeAlias = SpanKindInput | MetricsInput | ErrorInfoInput


def run(input_value: Input) -> LLMMetrics | DDLLMObsError | str | None:
    if input_value["function"] == "span_kind":
        return span_kind(input_value)
    if input_value["function"] == "metrics":
        return metrics(input_value)
    return error_info(input_value)


def span_kind(input_value: SpanKindInput) -> str:
    return _LOGGER._get_datadog_span_kind(  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/datadog/datadog_llm_obs.py#L842
        input_value["call_type"], input_value["parent_id"]
    )


def metrics(input_value: MetricsInput) -> LLMMetrics:
    return _LOGGER._assemble_metrics(  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/datadog/datadog_llm_obs.py#L730
        cast("StandardLoggingPayload", input_value["payload"]),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L3570
        input_value["tool_output_tokens"],
    )


def error_info(input_value: ErrorInfoInput) -> DDLLMObsError | None:
    return _LOGGER._assemble_error_info(  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/datadog/datadog_llm_obs.py#L697
        cast("StandardLoggingPayload", input_value["payload"]),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L3570
    )
