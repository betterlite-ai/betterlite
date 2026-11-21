import asyncio
from typing import Final, Literal, NotRequired, Required, TypeAlias, TypedDict

from litellm.proxy._types import BaseDailySpendTransaction, SpendLogsPayload
from litellm.proxy.db.baseline_accounting import DailyBaselineAttribution
from litellm.proxy.db.db_spend_update_writer import DBSpendUpdateWriter
from litellm.proxy.utils import PrismaClient
from pydantic import JsonValue, TypeAdapter

Entity = Literal["user", "team", "org", "tag", "end_user", "agent"]
CommonKind = Literal["user", "team", "org", "request_tags", "end_user", "agent"]
TargetEntity = Literal["user", "team", "org", "end_user", "agent", "tag"]

# _common_add_spend_log_transaction_to_daily_transaction calls the tag entity "request_tags".
COMMON_KINDS: Final[dict[Entity, CommonKind]] = {
    "user": "user",
    "team": "team",
    "org": "org",
    "tag": "request_tags",
    "end_user": "end_user",
    "agent": "agent",
}


class CaseInput(TypedDict, total=False):
    operation: Required[Literal["status", "common", "baseline"]]
    payload: Required[str]
    entity: Entity
    prices: dict[str, dict[str, JsonValue]]
    endpoints: dict[str, str]


BaseView = TypedDict(
    "BaseView",
    {
        "date": str,
        "api_key": str,
        "model": str | None,
        "model_group": str | None,
        "mcp_namespaced_tool_name": str | None,
        "custom_llm_provider": str | None,
        "endpoint": str | None,
        "prompt_tokens": int,
        "completion_tokens": int,
        "cache_read_input_tokens": int,
        "cache_creation_input_tokens": int,
        "compression_saved_tokens": int,
        "compression_savings_spend": float,
        "prompt_caching_savings_spend": float,
        "gateway_injected_caching_savings_spend": float,
        "autorouter_savings_spend": NotRequired[float],
        "spend": float,
        "api_requests": int,
        "successful_requests": int,
        "failed_requests": int,
        "total_response_time_ms": NotRequired[int],
        "timed_requests": NotRequired[int],
    },
)


class TargetView(TypedDict):
    entity: TargetEntity
    entity_id: str | None


class AttributionView(TypedDict):
    date: str
    api_key: str
    model: str | None
    custom_llm_provider: str | None
    model_group: str | None
    endpoint: str | None
    mcp_namespaced_tool_name: str | None
    targets: tuple[TargetView, ...]


class StatusAnswer(TypedDict):
    status: Literal["success", "failure"]
    raised: bool


class CommonAnswer(TypedDict):
    base: BaseView | None
    raised: bool


class BaselineAnswer(TypedDict):
    attribution: AttributionView | None
    raised: bool


Answer: TypeAlias = StatusAnswer | CommonAnswer | BaselineAnswer

PAYLOAD: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
SPEND_LOG: Final[TypeAdapter[SpendLogsPayload]] = TypeAdapter(SpendLogsPayload)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L4219
BASE: Final[TypeAdapter[BaseView]] = TypeAdapter(BaseView)
ATTRIBUTION: Final[TypeAdapter[AttributionView]] = TypeAdapter(AttributionView)


class FakeClient(PrismaClient):
    def __init__(self) -> None: ...


class CaseWriter(DBSpendUpdateWriter):
    async def common(
        self, payload: dict[str, JsonValue], entity: Entity
    ) -> BaseDailySpendTransaction | None:
        return await self._common_add_spend_log_transaction_to_daily_transaction(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/db/db_spend_update_writer.py#L2683
            payload, FakeClient(), COMMON_KINDS[entity]
        )

    async def baseline(
        self, payload: SpendLogsPayload
    ) -> DailyBaselineAttribution | None:
        return await self._baseline_daily_attribution(payload, FakeClient())  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/db/db_spend_update_writer.py#L850


def status_answer(input_value: CaseInput) -> StatusAnswer:
    payload = PAYLOAD.validate_json(input_value["payload"])
    status = FakeClient().get_request_status(payload)  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/utils.py#L4592
    return StatusAnswer(status=status, raised=False)


def common_answer(input_value: CaseInput) -> CommonAnswer:
    entity = input_value.get("entity")
    if entity is None:
        raise LookupError("a common operation needs an entity")
    payload = PAYLOAD.validate_json(input_value["payload"])
    try:
        transaction = asyncio.run(CaseWriter().common(payload, entity))
    except (AttributeError, KeyError, TypeError, ValueError):
        return CommonAnswer(base=None, raised=True)
    if transaction is None:
        return CommonAnswer(base=None, raised=False)
    return CommonAnswer(base=BASE.validate_python(transaction), raised=False)


def baseline_answer(input_value: CaseInput) -> BaselineAnswer:
    payload = SPEND_LOG.validate_json(input_value["payload"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L4219
    try:
        record = asyncio.run(CaseWriter().baseline(payload))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L4219
    except (AttributeError, KeyError, TypeError, ValueError):
        return BaselineAnswer(attribution=None, raised=True)
    if record is None:
        return BaselineAnswer(attribution=None, raised=False)
    return BaselineAnswer(
        attribution=ATTRIBUTION.validate_json(record.model_dump_json()), raised=False
    )


def run(input_value: CaseInput) -> Answer:
    operation = input_value["operation"]
    if operation == "status":
        return status_answer(input_value)
    if operation == "common":
        return common_answer(input_value)
    return baseline_answer(input_value)
