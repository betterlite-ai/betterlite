from enum import Enum, StrEnum
from typing import Final, Unpack

from pydantic import BaseModel, Field, JsonValue
from typing_extensions import TypedDict

from betterlite.types.utils.base import DumpOptions


class ServiceMetrics(Enum):
    COUNTER = "counter"
    HISTOGRAM = "histogram"
    GAUGE = "gauge"


class ServiceTypes(StrEnum):
    REDIS = "redis"
    DB = "postgres"
    BATCH_WRITE_TO_DB = "batch_write_to_db"
    RESET_BUDGET_JOB = "reset_budget_job"
    LITELLM = "self"
    ROUTER = "router"
    AUTH = "auth"
    PROXY_PRE_CALL = "proxy_pre_call"
    POD_LOCK_MANAGER = "pod_lock_manager"
    # ast-grep-ignore: no-long-names
    IN_MEMORY_DAILY_SPEND_UPDATE_QUEUE = "in_memory_daily_spend_update_queue"
    REDIS_DAILY_SPEND_UPDATE_QUEUE = "redis_daily_spend_update_queue"
    # ast-grep-ignore: no-long-names
    REDIS_DAILY_END_USER_SPEND_UPDATE_QUEUE = "redis_daily_end_user_spend_update_queue"
    # ast-grep-ignore: no-long-names
    REDIS_DAILY_ORG_SPEND_UPDATE_QUEUE = "redis_daily_org_spend_update_queue"
    # ast-grep-ignore: no-long-names
    REDIS_DAILY_TEAM_SPEND_UPDATE_QUEUE = "redis_daily_team_spend_update_queue"
    # ast-grep-ignore: no-long-names
    REDIS_DAILY_AGENT_SPEND_UPDATE_QUEUE = "redis_daily_agent_spend_update_queue"
    # ast-grep-ignore: no-long-names
    REDIS_DAILY_TAG_SPEND_UPDATE_QUEUE = "redis_daily_tag_spend_update_queue"
    # spend update queue, current spend of key, user, team
    IN_MEMORY_SPEND_UPDATE_QUEUE = "in_memory_spend_update_queue"
    REDIS_SPEND_UPDATE_QUEUE = "redis_spend_update_queue"
    # ast-grep-ignore: no-long-names
    REDIS_WINDOW_SPEND_UPDATE_QUEUE = "redis_window_spend_update_queue"


class ServiceConfig(TypedDict):
    metrics: list[ServiceMetrics]


DEFAULT_SERVICE_CONFIGS: Final[dict[str, ServiceConfig]] = {
    ServiceTypes.REDIS.value: {"metrics": [ServiceMetrics.COUNTER, ServiceMetrics.HISTOGRAM]},
    ServiceTypes.DB.value: {"metrics": [ServiceMetrics.COUNTER, ServiceMetrics.HISTOGRAM]},
    ServiceTypes.BATCH_WRITE_TO_DB.value: {
        "metrics": [ServiceMetrics.COUNTER, ServiceMetrics.HISTOGRAM]
    },
    ServiceTypes.RESET_BUDGET_JOB.value: {
        "metrics": [ServiceMetrics.COUNTER, ServiceMetrics.HISTOGRAM]
    },
    ServiceTypes.LITELLM.value: {"metrics": [ServiceMetrics.COUNTER, ServiceMetrics.HISTOGRAM]},
    ServiceTypes.ROUTER.value: {"metrics": [ServiceMetrics.COUNTER, ServiceMetrics.HISTOGRAM]},
    ServiceTypes.AUTH.value: {"metrics": [ServiceMetrics.COUNTER, ServiceMetrics.HISTOGRAM]},
    ServiceTypes.PROXY_PRE_CALL.value: {
        "metrics": [ServiceMetrics.COUNTER, ServiceMetrics.HISTOGRAM]
    },
    ServiceTypes.POD_LOCK_MANAGER.value: {"metrics": [ServiceMetrics.GAUGE]},
    ServiceTypes.IN_MEMORY_DAILY_SPEND_UPDATE_QUEUE.value: {"metrics": [ServiceMetrics.GAUGE]},
    ServiceTypes.REDIS_DAILY_SPEND_UPDATE_QUEUE.value: {"metrics": [ServiceMetrics.GAUGE]},
    ServiceTypes.REDIS_DAILY_END_USER_SPEND_UPDATE_QUEUE.value: {"metrics": [ServiceMetrics.GAUGE]},
    ServiceTypes.REDIS_DAILY_AGENT_SPEND_UPDATE_QUEUE.value: {"metrics": [ServiceMetrics.GAUGE]},
    ServiceTypes.IN_MEMORY_SPEND_UPDATE_QUEUE.value: {"metrics": [ServiceMetrics.GAUGE]},
    ServiceTypes.REDIS_SPEND_UPDATE_QUEUE.value: {"metrics": [ServiceMetrics.GAUGE]},
}


class ServiceEventMetadata(TypedDict, total=False):
    gauge_labels: str | None
    gauge_value: float | None


class ServiceLoggerPayload(BaseModel):
    is_error: bool = Field(description="did an error occur")
    error: str | None = Field(default=None, description="what was the error")
    service: ServiceTypes = Field(description="who is this for? - postgres/redis")
    duration: float = Field(description="How long did the request take?")
    call_type: str = Field(description="The call of the service, being made")
    caller: str | None = Field(
        default=None,
        description="The litellm call chain that made the service call, innermost first",
    )
    event_metadata: dict[str, JsonValue] | None = Field(
        description="The metadata logged during service success/failure"
    )

    def to_json(self, **kwargs: Unpack[DumpOptions]) -> dict[str, JsonValue]:
        dumped: dict[str, JsonValue] = self.model_dump(**kwargs)
        return dumped
