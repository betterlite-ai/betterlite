from datetime import datetime
from typing import Literal, NotRequired, TypeAlias, TypedDict, cast

from litellm.proxy._types import SpendLogsPayload
from litellm.proxy.db.db_transaction_queue.window_spend_update_queue import (
    WindowSpendTransaction,
    WindowSpendUpdateQueue,
    WindowSpendWirePayload,
    to_wire_payload,
)
from litellm.proxy.db.model_usage_rollup import ModelUsageTransaction, build_model_usage_transaction


class WindowStep(TypedDict):
    updates: list[list[WindowSpendTransaction]]


class WireStep(TypedDict):
    transaction: WindowSpendTransaction


# the fields build_model_usage_transaction reads, typed as SpendLogsPayload declares them, so nothing here carries Any
class Payload(TypedDict):
    startTime: datetime | str
    metadata: str
    model: str
    model_group: str | None
    custom_llm_provider: str | None
    request_tags: str
    spend: float
    prompt_tokens: int
    completion_tokens: int
    status: Literal["success", "failure"]


class UsageStep(TypedDict):
    payload: Payload


class Input(TypedDict):
    window_transactions: NotRequired[WindowStep]
    wire_payload: NotRequired[WireStep]
    model_usage: NotRequired[UsageStep]


class UsageKey(TypedDict):
    date: str
    model_group: str
    model: str
    custom_llm_provider: str
    task_type: str


class UsageResult(TypedDict):
    key: UsageKey
    spend: float
    prompt_tokens: int
    completion_tokens: int
    successful: bool


Result: TypeAlias = tuple[WindowSpendTransaction, ...] | WindowSpendWirePayload | UsageResult | None


def usage_result(transaction: ModelUsageTransaction) -> UsageResult:
    key = transaction.key
    return UsageResult(
        key=UsageKey(
            date=key.date,
            model_group=key.model_group,
            model=key.model,
            custom_llm_provider=key.custom_llm_provider,
            task_type=key.task_type,
        ),
        spend=transaction.spend,
        prompt_tokens=transaction.prompt_tokens,
        completion_tokens=transaction.completion_tokens,
        successful=transaction.successful,
    )


def model_usage(step: UsageStep) -> UsageResult | None:
    transaction = build_model_usage_transaction(
        cast(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L4219-L4253
            SpendLogsPayload, step["payload"]
        )
    )
    if transaction is None:
        return None
    return usage_result(transaction)


def run(input_value: Input) -> dict[str, Result]:
    results: dict[str, Result] = {}
    window_step = input_value.get("window_transactions")
    if window_step is not None:
        results["window_transactions"] = (
            WindowSpendUpdateQueue.get_aggregated_window_spend_transactions(window_step["updates"])
        )
    wire_step = input_value.get("wire_payload")
    if wire_step is not None:
        results["wire_payload"] = to_wire_payload(wire_step["transaction"])
    usage_step = input_value.get("model_usage")
    if usage_step is not None:
        results["model_usage"] = model_usage(usage_step)
    return results
