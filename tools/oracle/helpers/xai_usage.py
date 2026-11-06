from decimal import Decimal
from typing import TypeAlias, cast
from unittest.mock import patch

from litellm.llms.xai import cost_calculator
from litellm.llms.xai.chat.transformation import (
    XAIChatConfig,
    _usage_restated_from_xai_ticks,  # pyright: ignore[reportPrivateUsage]
)
from litellm.llms.xai.common_utils import xai_reported_cost_in_usd
from litellm.types.utils import CompletionTokensDetailsWrapper, ModelInfo, Usage

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class RecordingPricing:
    def __init__(self) -> None:
        self.seen: list[Usage] = []

    def __call__(
        self, *, model: str, usage: Usage, custom_llm_provider: str
    ) -> tuple[float, float]:
        assert (model, custom_llm_provider) == ("oracle-model", "xai")  # noqa: S101
        self.seen.append(usage)
        return 0.11, 0.22


def money(value: float | None) -> str | None:
    if value is None:
        return None
    return format(Decimal(repr(float(value))).normalize(), "f")


def build_usage(fields: dict[str, Json]) -> Usage:
    reasoning = cast(int | None, fields.get("reasoning_tokens"))
    details = None
    if reasoning is not None:
        details = CompletionTokensDetailsWrapper.model_construct(reasoning_tokens=reasoning)
    usage = Usage.model_construct(
        prompt_tokens=cast(int, fields.get("prompt_tokens", 0)),
        completion_tokens=cast(int, fields.get("completion_tokens", 0)),
        total_tokens=cast(int, fields.get("total_tokens", 0)),
        prompt_tokens_details=None,
        completion_tokens_details=details,
    )
    cost = fields.get("cost")
    if cost is not None:
        usage.cost = cost  # pyright: ignore[reportAttributeAccessIssue]
    return usage


def counts(usage: Usage) -> Json:
    return {
        "prompt_tokens": usage.prompt_tokens,
        "completion_tokens": usage.completion_tokens,
        "total_tokens": usage.total_tokens,
    }


def run_ticks(value: dict[str, Json]) -> Json:
    ticks = value["ticks"]
    usage = build_usage({})
    usage.cost_in_usd_ticks = ticks  # pyright: ignore[reportAttributeAccessIssue]
    restated = _usage_restated_from_xai_ticks(usage)
    restated_cost = None
    if restated is not None:
        restated_cost = cast(float | None, getattr(restated, "cost", None))
    return {"cost": money(xai_reported_cost_in_usd(ticks)), "restated_cost": money(restated_cost)}


def run_cost(value: dict[str, Json]) -> Json:
    usage = build_usage(cast(dict[str, Json], value["usage"]))
    recorder = RecordingPricing()
    with patch.object(cost_calculator, "generic_cost_per_token", recorder):
        prompt_cost, completion_cost = cost_calculator.cost_per_token("oracle-model", usage)
    delegate = None
    if recorder.seen:
        adjusted = recorder.seen[0]
        delegate = {
            **cast(dict[str, Json], counts(adjusted)),
            "has_completion_details": bool(adjusted.completion_tokens_details),
        }
    return {
        "prompt_cost": money(prompt_cost),
        "completion_cost": money(completion_cost),
        "delegate": delegate,
    }


def run_reconcile(value: dict[str, Json]) -> Json:
    usage = build_usage(cast(dict[str, Json], value["usage"]))
    steps: list[Json] = []
    for _ in range(2):
        XAIChatConfig.fold_reasoning_tokens_into_completion(usage)
        XAIChatConfig._normalize_openai_compatible_usage_totals(usage)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        steps.append(counts(usage))
    return {"once": steps[0], "twice": steps[1]}


def run_search(value: dict[str, Json]) -> Json:
    usage = build_usage(cast(dict[str, Json], value["usage"]))
    details = cast(dict[str, Json] | None, value.get("details"))
    cost_calculator.apply_server_side_tool_usage_details_to_usage(usage, details)
    search_costs = value.get("search_costs")
    model_info = {} if search_costs is None else {"search_context_cost_per_query": search_costs}
    search_cost = cost_calculator.cost_per_web_search_request(usage, cast(ModelInfo, model_info))
    prompt_details = usage.prompt_tokens_details
    requests = None if prompt_details is None else prompt_details.web_search_requests
    return {
        "attached": cast(Json, getattr(usage, "server_side_tool_usage_details", None)),
        "web_search_requests": requests,
        "search_cost": money(search_cost),
    }


def run(value: dict[str, Json]) -> Json:
    action = cast(str, value["action"])
    if action == "ticks":
        return run_ticks(value)
    if action == "cost":
        return run_cost(value)
    if action == "reconcile":
        return run_reconcile(value)
    return run_search(value)
