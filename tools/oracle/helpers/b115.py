import asyncio
from datetime import UTC, datetime, tzinfo
from typing import ClassVar, NotRequired, Self, TypedDict

import litellm
from litellm.integrations import s3_v2
from litellm.integrations.s3_v2 import S3Logger
from litellm.types.integrations.s3_v2 import s3BatchLoggingElement
from litellm.types.utils import StandardAuditLogPayload
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override


class Element(TypedDict):
    payload: dict[str, JsonValue]
    s3_object_key: str
    s3_object_download_filename: str


class Input(TypedDict):
    global_params: dict[str, JsonValue]
    record: StandardAuditLogPayload
    now: str
    override_params: NotRequired[dict[str, JsonValue]]


ELEMENT: TypeAdapter[Element] = TypeAdapter(Element)


class FrozenDatetime(datetime):
    instant: ClassVar[datetime] = datetime.fromtimestamp(0, UTC)

    @classmethod
    @override
    def now(cls, tz: tzinfo | None = None) -> Self:
        if tz is None:
            return cls.fromtimestamp(cls.instant.timestamp())
        return cls.fromtimestamp(cls.instant.timestamp(), tz)


def _build_logger(input_value: Input) -> S3Logger:
    override = input_value.get("override_params")
    if override is None:
        return S3Logger()
    return S3Logger(s3_callback_params_override=override)


def _stop_flush_tasks() -> None:
    # LiteLLM keeps no handle on its flush loop, so the loop's task set finds it again
    current: asyncio.Task[None] | None = asyncio.current_task()  # pyright: ignore[reportAny]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/s3_v2.py#L232
    tasks: set[asyncio.Task[None]] = asyncio.all_tasks()  # pyright: ignore[reportAny]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/s3_v2.py#L232
    for pending in tasks:
        if pending is not current:
            pending.cancel()


async def execute(input_value: Input) -> JsonValue:
    FrozenDatetime.instant = datetime.fromisoformat(input_value["now"])
    previous_params: dict[str, JsonValue] | None = litellm.s3_callback_params  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L444
    try:
        litellm.s3_callback_params = input_value["global_params"]
        s3_v2.datetime = FrozenDatetime  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/s3_v2.py#L490
        logger = _build_logger(input_value)
        await logger.async_log_audit_log_event(input_value["record"])
        queued: list[s3BatchLoggingElement] = logger.log_queue
        element = ELEMENT.validate_json(queued[-1].model_dump_json())
        return {
            "payload": element["payload"],
            "s3_object_key": element["s3_object_key"],
            "s3_object_download_filename": element["s3_object_download_filename"],
        }
    finally:
        s3_v2.datetime = datetime  # type: ignore[attr-defined]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/s3_v2.py#L490
        litellm.s3_callback_params = previous_params
        _stop_flush_tasks()


def run(input_value: Input) -> JsonValue:
    coroutine = execute(input_value)
    return asyncio.run(coroutine)
