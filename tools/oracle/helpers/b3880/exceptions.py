from typing import Literal, TypedDict

from litellm.exceptions import BudgetExceededError, RateLimitError
from litellm.proxy.common_utils.proxy_rate_limit_error import ProxyRateLimitError
from typing_extensions import override


class ExceptionCase(TypedDict):
    kind: Literal["scripted", "rate_limit", "budget_exceeded", "proxy_rate_limit"]
    message: str | None
    status_code: str | None
    code: str | None
    llm_provider: str | None
    category: str | None
    rate_limit_type: str | None
    legacy_class_name: str | None


class ScriptedError(Exception):
    status_code: str | None
    code: str | None
    llm_provider: str | None
    category: str | None
    rate_limit_type: str | None

    @override
    def __init__(self, case: ExceptionCase) -> None:
        message = case.get("message")
        if message is None:
            super().__init__()
        else:
            super().__init__(message)
        self.status_code = case.get("status_code")
        self.code = case.get("code")
        self.llm_provider = case.get("llm_provider")
        self.category = case.get("category")
        self.rate_limit_type = case.get("rate_limit_type")
        self.prometheus_exception_class_name = case.get("legacy_class_name")


def message_text(case: ExceptionCase) -> str:
    message = case.get("message")
    if message is None:
        return ""
    return message


def build_exception(case: ExceptionCase) -> Exception:
    kind = case["kind"]
    if kind == "scripted":
        return ScriptedError(case)
    if kind == "rate_limit":
        category = case.get("category")
        return RateLimitError(
            message=message_text(case),
            llm_provider=case.get("llm_provider"),
            model="",
            category="vendor_rate_limit" if category is None else category,
            rate_limit_type=case.get("rate_limit_type"),
        )
    if kind == "budget_exceeded":
        return BudgetExceededError(
            current_cost=0.0,
            max_budget=0.0,
            message=message_text(case),
            llm_provider=case.get("llm_provider"),
        )
    proxy_category = case.get("category")
    return ProxyRateLimitError(
        detail=message_text(case),
        category="litellm_rate_limit" if proxy_category is None else proxy_category,
        rate_limit_type=case.get("rate_limit_type"),
        llm_provider=case.get("llm_provider"),
    )
