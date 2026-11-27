from collections.abc import Sequence
from enum import Enum
from typing import ClassVar, Final

from pydantic import ConfigDict, Field
from typing_extensions import ReadOnly, TypedDict

from betterlite._router_types_groups import RouterErrors
from betterlite.types.utils.base import ResponseObject


class RouterGeneralSettings(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    async_only_mode: bool = Field(default=False)
    pass_through_all_models: bool = Field(default=False)


class RouterBasicRateLimitError(ValueError):
    def __init__(self, model: str) -> None:
        self.model = model
        super().__init__(f"{RouterErrors.no_deployments_available.value}.")


class RouterErrorTypes(str, Enum):
    rate_limit_error = "rate_limit_error"
    all_deployments_in_cooldown = "all_deployments_in_cooldown"


class RouterRateLimitError(ValueError):
    def __init__(
        self, model: str, *, cooldown_time: float, enable_pre_call_checks: bool,
        cooldown_list: list[str], model_ids: Sequence[str] = (),
    ) -> None:
        self.model = model
        self.cooldown_time = cooldown_time
        self.enable_pre_call_checks = enable_pre_call_checks
        self.cooldown_list = cooldown_list
        cooldown_ids: Final = frozenset(cooldown_list)
        self.all_deployments_in_cooldown = bool(model_ids) and frozenset(model_ids) <= cooldown_ids
        self.type = RouterErrorTypes.rate_limit_error.value
        reason = ""
        if self.all_deployments_in_cooldown:
            self.type = RouterErrorTypes.all_deployments_in_cooldown.value
            reason = f" {RouterErrors.all_deployments_in_cooldown.value}."
        super().__init__(
            f"{RouterErrors.no_deployments_available.value}, Try again in {cooldown_time} seconds."
            f"{reason} Passed model={model}. pre-call-checks={enable_pre_call_checks}, "
            f"cooldown_list={cooldown_list}"
        )


class RouterModelGroupAliasItem(TypedDict):
    model: str
    hidden: bool


class RetryAttemptRecord(TypedDict):
    model_group: ReadOnly[str | None]
    deployment_id: ReadOnly[str | None]
    exception_type: ReadOnly[str]
    exception_string: ReadOnly[str]
    attempted_retries: ReadOnly[int | None]


VALID_LITELLM_ENVIRONMENTS: Final[list[str]] = ["development", "staging", "production"]


class RoutingStrategy(Enum):
    LEAST_BUSY = "least-busy"
    LATENCY_BASED = "latency-based-routing"
    COST_BASED = "cost-based-routing"
    USAGE_BASED_ROUTING_V2 = "usage-based-routing-v2"
    USAGE_BASED_ROUTING = "usage-based-routing"
    PROVIDER_BUDGET_LIMITING = "provider-budget-routing"


class RouterCacheEnum(Enum):
    TPM = "global_router:{id}:{model}:tpm:{current_minute}"
    RPM = "global_router:{id}:{model}:rpm:{current_minute}"
    ITPM = "global_router:{id}:{model}:itpm:{current_minute}"
    OTPM = "global_router:{id}:{model}:otpm:{current_minute}"


class GenericBudgetWindowDetails(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    budget_start: float
    spend_key: str
    start_time_key: str
    ttl_seconds: int
