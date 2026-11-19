import asyncio
from collections.abc import Mapping
from typing import Final, Literal, Required, TypedDict

from litellm.proxy._types import (
    Litellm_EntityType,
    SpendUpdateQueueItem,
)
from litellm.proxy.db.autorouter_session_rollup import (
    AutoRouterTurnTransaction,
    build_autorouter_turn_transaction,
)
from litellm.proxy.db.db_spend_update_writer import DBSpendUpdateWriter
from litellm.proxy.db.db_transaction_queue.spend_update_queue import SpendUpdateQueue
from litellm.proxy.utils import PrismaClient
from pydantic import JsonValue
from typing_extensions import override

EntityKind = Literal["team", "org", "project", "agent", "tag", "model_access_group"]

ENTITY_NAMES: Final[dict[Litellm_EntityType, str]] = {
    Litellm_EntityType.KEY: "key",
    Litellm_EntityType.USER: "user",
    Litellm_EntityType.END_USER: "end_user",
    Litellm_EntityType.TEAM: "team",
    Litellm_EntityType.TEAM_MEMBER: "team_member",
    Litellm_EntityType.ORGANIZATION: "organization",
    Litellm_EntityType.ORGANIZATION_MEMBER: "organization_member",
    Litellm_EntityType.PROJECT: "project",
    Litellm_EntityType.TAG: "tag",
    Litellm_EntityType.AGENT: "agent",
    Litellm_EntityType.MODEL_ACCESS_GROUP: "model_access_group",
    Litellm_EntityType.PROXY: "proxy",
}


class Step(TypedDict, total=False):
    kind: Required[Literal["turn", "enqueue"]]
    payload: dict[str, JsonValue]
    metadata: dict[str, JsonValue]
    saved_spend: float
    entity: EntityKind
    response_cost: float | None
    team_id: str | None
    org_id: str | None
    project_id: str | None
    agent_id: str | None
    user_id: str | None
    request_tags: str | None
    attributed: list[str] | None
    served_model_id: str | None
    deployments: dict[str, dict[str, JsonValue]] | None
    fail_at: int | None
    database: bool


class Input(TypedDict):
    steps: list[Step]


class TurnView(TypedDict):
    api_key: str
    session_id: str
    router_name: str
    router_type: str
    model: str
    turn_at: str
    total_tokens: int
    spend: str
    saved_spend: str
    classifier_cost: str
    covered: bool
    cache_hit: bool
    cache_ttl_seconds: int | None
    cache_touched: bool
    tier: str | None
    baseline_model: str | None
    savings_estimated_turns: int
    savings_estimated_actual_spend: str
    savings_estimated_saved_spend: str
    user_id: str


class TurnAnswer(TypedDict):
    record: TurnView | None


class QueuedItem(TypedDict):
    entity_type: str
    entity_id: str
    response_cost: str | None


class EnqueueAnswer(TypedDict):
    queued: list[QueuedItem]
    raised: bool


Answer = TurnAnswer | EnqueueAnswer


class QueueUnavailableError(Exception):
    pass


class RecordingQueue(SpendUpdateQueue):
    def __init__(self, fail_at: int | None) -> None:
        self.items: list[SpendUpdateQueueItem] = []
        self.fail_at = fail_at

    @override
    async def add_update(self, update: SpendUpdateQueueItem) -> None:
        if self.fail_at is not None and len(self.items) == self.fail_at:
            raise QueueUnavailableError
        self.items.append(update)


class DeploymentRouter:
    def __init__(self, deployments: dict[str, dict[str, JsonValue]]) -> None:
        self.deployments = deployments

    def get_model_info(self, id: str) -> Mapping[str, JsonValue] | None:  # noqa: A002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/db/db_spend_update_writer.py#L408-L409
        if id not in self.deployments:
            return None
        return self.deployments[id]


class FakeClient(PrismaClient):
    def __init__(self) -> None: ...


class CaseWriter(DBSpendUpdateWriter):
    def __init__(self, queue: RecordingQueue) -> None:
        super().__init__()
        self.spend_update_queue = queue

    async def run_step(
        self,
        step: Step,
        cost: float | None,
        client: PrismaClient | None,
        router: DeploymentRouter | None,
    ) -> None:
        match step.get("entity"):
            case "team":
                await self._update_team_db(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/db/db_spend_update_writer.py#L1232
                    response_cost=cost,
                    team_id=step.get("team_id"),
                    user_id=step.get("user_id"),
                    prisma_client=client,
                )
            case "org":
                await self._update_org_db(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/db/db_spend_update_writer.py#L1286
                    response_cost=cost,
                    org_id=step.get("org_id"),
                    user_id=step.get("user_id"),
                    prisma_client=client,
                )
            case "project":
                await self._update_project_db(
                    response_cost=cost,
                    project_id=step.get("project_id"),
                    prisma_client=client,
                )
            case "agent":
                await self._update_agent_db(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/db/db_spend_update_writer.py#L1352
                    response_cost=cost,
                    agent_id=step.get("agent_id"),
                    prisma_client=client,
                )
            case "tag":
                await self._update_tag_db(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/db/db_spend_update_writer.py#L1379
                    response_cost=cost,
                    request_tags=step.get("request_tags"),
                    prisma_client=client,
                )
            case None:
                raise LookupError("an enqueue step needs an entity")
            case "model_access_group":
                await self._update_model_access_group_db(
                    response_cost=cost,
                    request_model_access_groups=step.get("attributed"),
                    served_model_id=step.get("served_model_id"),
                    prisma_client=client,
                    router=router,
                )


def turn_view(record: AutoRouterTurnTransaction) -> TurnView:
    return TurnView(
        api_key=record.api_key,
        session_id=record.session_id,
        router_name=record.router_name,
        router_type=record.router_type,
        model=record.model,
        turn_at=record.turn_at.isoformat(),
        total_tokens=record.total_tokens,
        spend=f"{record.spend:.10f}",
        saved_spend=f"{record.saved_spend:.10f}",
        classifier_cost=f"{record.classifier_cost:.10f}",
        covered=record.covered,
        cache_hit=record.cache_hit,
        cache_ttl_seconds=record.cache_ttl_seconds,
        cache_touched=record.cache_touched,
        tier=record.tier,
        baseline_model=record.baseline_model,
        savings_estimated_turns=record.savings_estimated_turns,
        savings_estimated_actual_spend=f"{record.savings_estimated_actual_spend:.10f}",
        savings_estimated_saved_spend=f"{record.savings_estimated_saved_spend:.10f}",
        user_id=record.user_id,
    )


def turn_answer(step: Step) -> TurnAnswer:
    payload = step.get("payload")
    if payload is None:
        raise LookupError("a turn step needs a payload")
    metadata = step.get("metadata")
    saved_spend = step.get("saved_spend")
    record = build_autorouter_turn_transaction(
        payload,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L4219-L4253
        {} if metadata is None else metadata,
        0.0 if saved_spend is None else saved_spend,
    )
    return TurnAnswer(record=None if record is None else turn_view(record))


def queued_item(item: SpendUpdateQueueItem) -> QueuedItem:
    kind = item.get("entity_type")
    entity_id = item.get("entity_id")
    cost = item.get("response_cost")
    return QueuedItem(
        entity_type="" if kind is None else ENTITY_NAMES[kind],
        entity_id="" if entity_id is None else entity_id,
        response_cost=None if cost is None else f"{cost:.10f}",
    )


def enqueue_answer(step: Step) -> EnqueueAnswer:
    queue = RecordingQueue(step.get("fail_at"))
    writer = CaseWriter(queue)
    deployments = step.get("deployments")
    router = None if deployments is None else DeploymentRouter(deployments)
    client = None if step.get("database") is False else FakeClient()
    raised = False
    try:
        asyncio.run(writer.run_step(step, step.get("response_cost"), client, router))
    except (QueueUnavailableError, TypeError):
        raised = True
    return EnqueueAnswer(queued=[queued_item(item) for item in queue.items], raised=raised)


def run(input_value: Input) -> list[Answer]:
    answers: list[Answer] = []
    for step in input_value["steps"]:
        if step["kind"] == "turn":
            answers.append(turn_answer(step))
        else:
            answers.append(enqueue_answer(step))
    return answers
