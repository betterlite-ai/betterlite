import asyncio
import logging
from dataclasses import dataclass
from typing import Literal, NotRequired, TypedDict

from litellm.proxy.openai_files_endpoints.common_utils import (
    batch_cost_poller_is_active,
    ensure_batch_response_managed_file_ids,
    get_batch_from_database,
    resolve_input_file_id_to_unified,
    resolve_output_file_ids_to_unified,
    update_batch_in_database,
)
from pydantic import JsonValue

Operation = Literal[
    "resolve_input",
    "resolve_output",
    "ensure",
    "database_get",
    "database_update",
    "poller_active",
]


class Files(TypedDict):
    input_file_id: NotRequired[str | None]
    output_file_id: NotRequired[str | None]
    error_file_id: NotRequired[str | None]


class Plan(TypedDict):
    op: Operation
    files: NotRequired[Files]
    batch_id: NotRequired[str | None]
    unified_batch_id: NotRequired[str | None]


class Database(TypedDict):
    present: bool
    found: NotRequired[dict[str, str]]
    failures: NotRequired[list[str]]


class Input(TypedDict):
    plan: Plan
    database: Database


class HasFilter(TypedDict):
    has: str


class FindWhere(TypedDict):
    flat_model_file_ids: HasFilter


@dataclass
class BatchResponse:
    input_file_id: str | None
    output_file_id: str | None
    error_file_id: str | None


@dataclass
class ManagedFile:
    unified_file_id: str


class LookupFailedError(Exception):
    pass


class FileTable:
    def __init__(self, database: Database, events: list[JsonValue]) -> None:
        found = database.get("found")
        failures = database.get("failures")
        self._found: dict[str, str] = {} if found is None else found
        self._failures: list[str] = [] if failures is None else failures
        self._events = events

    async def find_first(self, where: FindWhere) -> ManagedFile | None:
        raw_id = where["flat_model_file_ids"]["has"]
        self._events.append({"event": "file_lookup", "raw_id": raw_id})
        if raw_id in self._failures:
            raise LookupFailedError(raw_id)
        unified_file_id = self._found.get(raw_id)
        if unified_file_id is None:
            return None
        return ManagedFile(unified_file_id=unified_file_id)


class FakeDb:
    def __init__(self, table: FileTable) -> None:
        self.litellm_managedfiletable = table


class FakeClient:
    def __init__(self, table: FileTable) -> None:
        self.db = FakeDb(table)


def batch_id(plan: Plan) -> str:
    value = plan.get("batch_id")
    if value is None:
        raise ValueError("the case gives no batch id")
    return value


def unified_batch_id(plan: Plan) -> str | Literal[False]:
    value = plan.get("unified_batch_id")
    if value is None:
        return False
    return value


def response_for(plan: Plan) -> BatchResponse:
    files = plan.get("files")
    if files is None:
        return BatchResponse(None, None, None)
    return BatchResponse(
        files.get("input_file_id"),
        files.get("output_file_id"),
        files.get("error_file_id"),
    )


def files_value(response: BatchResponse) -> dict[str, JsonValue]:
    return {
        "input_file_id": response.input_file_id,
        "output_file_id": response.output_file_id,
        "error_file_id": response.error_file_id,
    }


async def dispatch(
    plan: Plan,
    client: FakeClient | None,
    logger: logging.Logger,
) -> dict[str, JsonValue]:
    response = response_for(plan)
    match plan["op"]:
        case "resolve_input":
            await resolve_input_file_id_to_unified(response, client)
            return files_value(response)
        case "resolve_output":
            await resolve_output_file_ids_to_unified(response, client)
            return files_value(response)
        case "ensure":
            await ensure_batch_response_managed_file_ids(response, None, client, logger)
            return files_value(response)
        case "database_get":
            result: tuple[None, None] = await get_batch_from_database(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/openai_files_endpoints/common_utils.py#L1324
                batch_id(plan),
                unified_batch_id(plan),
                None,
                client,
                logger,
            )
            return {"db_batch_object": result[0], "response": result[1]}
        case "database_update":
            await update_batch_in_database(
                batch_id(plan),
                unified_batch_id(plan),
                response,
                None,
                client,
                logger,
            )
            return {"files": files_value(response), "written": False}
        case "poller_active":
            return {"active": batch_cost_poller_is_active()}


def run(input_value: Input) -> dict[str, JsonValue]:
    events: list[JsonValue] = []
    database = input_value["database"]
    table = FileTable(database, events)
    client: FakeClient | None = FakeClient(table) if database["present"] else None
    logger = logging.getLogger("LiteLLM")
    result = asyncio.run(dispatch(input_value["plan"], client, logger))
    return {"result": result, "events": events}
