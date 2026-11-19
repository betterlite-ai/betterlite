from typing import Literal, Required, TypedDict

from pydantic import JsonValue


class StepSpec(TypedDict, total=False):
    kind: Required[Literal["response", "error"]]
    value: JsonValue
    error_class: str
    message: str


class BudgetSpec(TypedDict, total=False):
    max_budget: float
    current_cost: float


class RetrySpec(TypedDict, total=False):
    num_retries: int | None
    num_retries_per_request: int | None


class CacheSpec(TypedDict, total=False):
    hit: JsonValue


class ObserveSpec(TypedDict, total=False):
    values: list[str]
    present: list[str]


class Input(TypedDict, total=False):
    mode: Literal["sync", "async"]
    request: Required[dict[str, JsonValue]]
    outcomes: list[StepSpec]
    budget: BudgetSpec
    retry: RetrySpec
    cache: CacheSpec
    observe: ObserveSpec


class ErrorReport(TypedDict):
    error_class: str


class Seen(TypedDict):
    values: dict[str, JsonValue]
    present: dict[str, bool]


class Answer(TypedDict):
    result: JsonValue
    error: ErrorReport | None
    calls: int
    seen: list[Seen]
