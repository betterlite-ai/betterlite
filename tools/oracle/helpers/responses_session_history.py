import asyncio
import logging
from typing import Final, TypedDict, TypeVar, overload

import litellm
from litellm.integrations.s3_v2 import S3Logger
from litellm.proxy import proxy_server
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.utils import PrismaClient
from litellm.responses.litellm_completion_transformation.transformation import (
    LiteLLMCompletionResponsesConfig as ResponsesConfig,
)
from litellm.types.utils import Message
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

ResultT = TypeVar("ResultT")
Completed = dict[str, JsonValue | list[JsonValue | Message]]

PROXY_LOGGER: Final = logging.getLogger("LiteLLM Proxy")
COMPLETED: Final = TypeAdapter(Completed)


class Input(TypedDict):
    previous_response_id: str
    database: bool
    disable_spend_logs: bool
    replies: list[list[dict[str, JsonValue]]]
    cold_storage: bool
    cold_objects: dict[str, dict[str, JsonValue] | None]
    cold_failures: list[str]
    request: dict[str, JsonValue]


class Merged(TypedDict):
    messages: list[JsonValue]
    litellm_trace_id: str | None


MERGED: Final = TypeAdapter(Merged)


class Effects:
    def __init__(self) -> None:
        self.queries: list[JsonValue] = []
        self.reads: list[JsonValue] = []
        self.sleeps: list[JsonValue] = []
        self.debug: list[JsonValue] = []


class ColdStorageFailureError(Exception):
    pass


class FakeWrapper(PrismaWrapper):
    def __init__(self, replies: list[list[dict[str, JsonValue]]], effects: Effects) -> None:
        self.replies = replies
        self.effects = effects

    async def query_raw(self, sql: str, *params: JsonValue) -> list[dict[str, JsonValue]]:
        self.effects.queries.append({"sql": " ".join(sql.split()), "request_id": params[0]})
        position = len(self.effects.queries) - 1
        if position < len(self.replies):
            return self.replies[position]
        return []


class FakePrismaClient(PrismaClient):
    def __init__(self, wrapper: FakeWrapper) -> None:
        self.db = wrapper


class FakeColdStorage(S3Logger):
    def __init__(self, input_value: Input, effects: Effects) -> None:
        self.stored_objects = input_value["cold_objects"]
        self.failing_keys = input_value["cold_failures"]
        self.recorded = effects

    @override
    async def get_proxy_server_request_from_cold_storage_with_object_key(  # ast-grep-ignore: no-long-names
        self,
        object_key: str,
    ) -> dict[str, JsonValue] | None:
        self.recorded.reads.append(object_key)
        if object_key in self.failing_keys:
            raise ColdStorageFailureError(object_key)
        if object_key in self.stored_objects:
            return self.stored_objects[object_key]
        return None


class FakeSleep:
    def __init__(self, effects: Effects) -> None:
        self.effects = effects

    @overload
    async def __call__(self, delay: float) -> None: ...

    @overload
    async def __call__(self, delay: float, result: ResultT) -> ResultT: ...

    async def __call__(self, delay: float, result: ResultT | None = None) -> ResultT | None:
        self.effects.sleeps.append(delay)
        return result


class DebugCapture(logging.Handler):
    def __init__(self, effects: Effects) -> None:
        super().__init__(level=logging.DEBUG)
        self.effects = effects

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.effects.debug.append(record.getMessage().partition("\n")[0])


async def merge(input_value: Input) -> Merged:
    request: dict[str, JsonValue] = dict(input_value["request"])
    completed: Completed = await ResponsesConfig.async_responses_api_session_handler(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L535-L538
        previous_response_id=input_value["previous_response_id"],
        litellm_completion_request=request,
    )
    return MERGED.validate_json(COMPLETED.dump_json(completed))


def run(input_value: Input) -> JsonValue:
    effects = Effects()
    PROXY_LOGGER.handlers = [DebugCapture(effects)]
    PROXY_LOGGER.setLevel(logging.DEBUG)
    litellm.callbacks = [FakeColdStorage(input_value, effects)]
    litellm.cold_storage_custom_logger = "s3_v2" if input_value["cold_storage"] else None
    proxy_server.disable_spend_logs = input_value["disable_spend_logs"]
    proxy_server.prisma_client = None
    if input_value["database"]:
        proxy_server.prisma_client = FakePrismaClient(FakeWrapper(input_value["replies"], effects))
    asyncio.sleep = FakeSleep(effects)
    merged = asyncio.run(merge(input_value))
    return {
        "queries": effects.queries,
        "reads": effects.reads,
        "sleeps": effects.sleeps,
        "debug": effects.debug,
        "messages": merged["messages"],
        "litellm_trace_id": merged["litellm_trace_id"],
    }
