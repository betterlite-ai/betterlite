import asyncio
import logging
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Literal, NotRequired, TypeAlias, TypedDict, cast

from litellm.proxy.openai_files_endpoints import common_utils
from pydantic import BaseModel, JsonValue

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


class BatchResponse(BaseModel):
    input_file_id: str | None
    output_file_id: str | None
    error_file_id: str | None
    status: str = "in_progress"


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


class ObjectTable:
    def __init__(self, events: list[JsonValue]) -> None:
        self._events = events
        self.written = False

    async def find_first(self, where: dict[str, str]) -> None:
        self._events.append(
            {"event": "object_lookup", "unified_object_id": where["unified_object_id"]}
        )

    async def update(self, where: dict[str, str], data: dict[str, JsonValue]) -> None:
        self._events.append(
            {
                "event": "object_update",
                "unified_object_id": where["unified_object_id"],
                "status": data.get("status"),
            }
        )
        self.written = True


class FakeDb:
    def __init__(self, files: FileTable, objects: ObjectTable) -> None:
        self.litellm_managedfiletable = files
        self.litellm_managedobjecttable = objects


class FakeClient:
    def __init__(self, files: FileTable, objects: ObjectTable) -> None:
        self.db = FakeDb(files, objects)


ResolveFiles: TypeAlias = Callable[[BatchResponse, FakeClient | None], Awaitable[None]]
EnsureFiles: TypeAlias = Callable[
    [BatchResponse, None, FakeClient | None, logging.Logger], Awaitable[None]
]
DatabaseGet: TypeAlias = Callable[
    [str, str | Literal[False], None, FakeClient | None, logging.Logger],
    Awaitable[tuple[None, None]],
]
DatabaseUpdate: TypeAlias = Callable[
    [str, str | Literal[False], BatchResponse, None, FakeClient | None, logging.Logger],
    Awaitable[None],
]

# LiteLLM leaves these parameters and returns unannotated; the casts give the shapes sent here
RESOLVE_INPUT, RESOLVE_OUTPUT, ENSURE, GET_BATCH, UPDATE = cast(ResolveFiles, common_utils.resolve_input_file_id_to_unified), cast(ResolveFiles, common_utils.resolve_output_file_ids_to_unified), cast(EnsureFiles, common_utils.ensure_batch_response_managed_file_ids), cast(DatabaseGet, common_utils.get_batch_from_database), cast(DatabaseUpdate, common_utils.update_batch_in_database)  # noqa: E501  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/openai_files_endpoints/common_utils.py#L1200


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
        return BatchResponse(input_file_id=None, output_file_id=None, error_file_id=None)
    return BatchResponse(
        input_file_id=files.get("input_file_id"),
        output_file_id=files.get("output_file_id"),
        error_file_id=files.get("error_file_id"),
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
            await RESOLVE_INPUT(response, client)
            return files_value(response)
        case "resolve_output":
            await RESOLVE_OUTPUT(response, client)
            return files_value(response)
        case "ensure":
            await ENSURE(response, None, client, logger)
            return files_value(response)
        case "database_get":
            result = await GET_BATCH(
                batch_id(plan),
                unified_batch_id(plan),
                None,
                client,
                logger,
            )
            return {"db_batch_object": result[0], "response": result[1]}
        case "database_update":
            await UPDATE(
                batch_id(plan),
                unified_batch_id(plan),
                response,
                None,
                client,
                logger,
            )
            written = client is not None and client.db.litellm_managedobjecttable.written
            return {"files": files_value(response), "written": written}
        case "poller_active":
            return {"active": common_utils.batch_cost_poller_is_active()}


def run(input_value: Input) -> dict[str, JsonValue]:
    events: list[JsonValue] = []
    database = input_value["database"]
    files = FileTable(database, events)
    objects = ObjectTable(events)
    client: FakeClient | None = FakeClient(files, objects) if database["present"] else None
    logger = logging.getLogger("LiteLLM")
    result = asyncio.run(dispatch(input_value["plan"], client, logger))
    return {"result": result, "events": events}
