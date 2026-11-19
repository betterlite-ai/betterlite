import asyncio
import logging
import time
from typing import NotRequired, TypedDict, Unpack

from litellm.integrations.compression_interception import handler as interception_handler
from litellm.integrations.compression_interception.handler import (
    CompressionInterceptionLogger,
)
from litellm.types.utils import CallTypes
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter
from typing_extensions import override

CALL_ID = "litellm_call_id"
GENERATED_ID = "<generated>"
COMPRESSED = "CompressionInterception: compressed request ["
SKIPPED = "CompressionInterception: compression skipped ["


class SettingsSource(TypedDict):
    litellm_settings: dict[str, JsonValue]
    callback_settings: dict[str, JsonValue]


class ScriptedResult(TypedDict):
    messages: list[JsonValue]
    tools: list[JsonValue]
    original_tokens: JsonValue
    compressed_tokens: JsonValue
    cache: dict[str, str]
    skipped_reason: NotRequired[str | None]


class CompressedScript(TypedDict):
    messages: list[JsonValue]
    tools: list[JsonValue]
    original_tokens: JsonValue
    compressed_tokens: JsonValue
    cache: dict[str, str]
    compression_skipped_reason: NotRequired[str]


class CompressArguments(TypedDict):
    messages: list[JsonValue]
    model: str
    call_type: CallTypes
    compression_trigger: int
    compression_target: int | None
    embedding_model: str | None
    embedding_model_params: dict[str, JsonValue] | None


class Step(TypedDict):
    request: dict[str, JsonValue]
    now: float
    settings: NotRequired[SettingsSource]
    call_type: NotRequired[str]
    lookup: NotRequired[str | None]
    compress: NotRequired[ScriptedResult]


class InterceptionCall(TypedDict):
    steps: list[Step]


class SettingsRecord(TypedDict):
    enabled: bool
    compression_trigger: int
    compression_target: int | None
    embedding_model: str | None
    embedding_model_params: dict[str, JsonValue] | None


class CompressCall(TypedDict):
    model: str
    call_type: str
    compression_trigger: int
    compression_target: int | None
    embedding_model: str | None
    embedding_model_params: dict[str, JsonValue] | None


class Diagnostic(TypedDict):
    level: str
    kind: str
    reason: str | None


class StepOutcome(TypedDict):
    settings: SettingsRecord
    skipped: bool
    request: dict[str, JsonValue]
    compress_calls: list[CompressCall]
    lookup: list[str]
    diagnostics: list[Diagnostic]


class InterceptionOutcome(TypedDict):
    steps: list[StepOutcome]


class OutcomeValue(TypedDict):
    value: InterceptionOutcome


class Clock:
    def __init__(self) -> None:
        self.now = 0.0

    def __call__(self) -> float:
        return self.now


class Recorder(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.DEBUG)
        self.diagnostics: list[Diagnostic] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        message = record.getMessage()
        level = record.levelname.lower()
        if message.startswith(COMPRESSED):
            self.diagnostics.append({"level": level, "kind": "compressed", "reason": None})
            return
        if message.startswith(SKIPPED):
            self.diagnostics.append(
                {"level": level, "kind": "skipped", "reason": skipped_reason(message)}
            )


def skipped_reason(message: str) -> str:
    listed = message.split("[reason=", maxsplit=1)[1]
    return listed.split(" ", maxsplit=1)[0]


def compressed(script: ScriptedResult) -> CompressedScript:
    result: CompressedScript = {
        "messages": script["messages"],
        "tools": script["tools"],
        "original_tokens": script["original_tokens"],
        "compressed_tokens": script["compressed_tokens"],
        "cache": script["cache"],
    }
    reason = script.get("skipped_reason")
    if reason is not None:
        result["compression_skipped_reason"] = reason
    return result


def settings_record(logger: CompressionInterceptionLogger) -> SettingsRecord:
    params = TypeAdapter(dict[str, JsonValue] | None).validate_python(
        logger.embedding_model_params
    )
    return {
        "enabled": logger.enabled,
        "compression_trigger": logger.compression_trigger,
        "compression_target": logger.compression_target,
        "embedding_model": logger.embedding_model,
        "embedding_model_params": params,
    }


class Scripted:
    def __init__(self) -> None:
        self.result: CompressedScript | None = None
        self.calls: list[CompressCall] = []

    def compress(self, **call: Unpack[CompressArguments]) -> CompressedScript:
        params = TypeAdapter(dict[str, JsonValue] | None).validate_python(
            call["embedding_model_params"]
        )
        self.calls.append(
            {
                "model": call["model"],
                "call_type": CallTypes(call["call_type"]).value,
                "compression_trigger": call["compression_trigger"],
                "compression_target": call["compression_target"],
                "embedding_model": call["embedding_model"],
                "embedding_model_params": params,
            }
        )
        scripted = self.result
        if scripted is None:
            raise RuntimeError("the case scripted no compression result for this call")
        self.result = None
        return scripted

    def take_calls(self) -> list[CompressCall]:
        calls = self.calls
        self.calls = []
        return calls


async def run_steps(
    call: InterceptionCall, clock: Clock, recorder: Recorder, scripted: Scripted
) -> list[StepOutcome]:
    steps: list[StepOutcome] = []
    logger: CompressionInterceptionLogger | None = None
    for step in call["steps"]:
        source = step.get("settings")
        if source is not None:
            logger = CompressionInterceptionLogger.initialize_from_proxy_config(
                source["litellm_settings"], source["callback_settings"]
            )
        if logger is None:
            raise ValueError("the first step names no settings")
        script = step.get("compress")
        if script is not None:
            scripted.result = compressed(script)
        clock.now = step["now"]
        request = dict(step["request"])
        previous = request.get(CALL_ID)
        call_type = step.get("call_type")
        typed = CallTypes(call_type) if call_type is not None else None
        returned = await logger.async_pre_call_deployment_hook(request, typed)
        if request.get(CALL_ID) != previous:
            request[CALL_ID] = GENERATED_ID
        cache = logger._get_cache(step.get("lookup"))  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/integrations/compression_interception/handler.py#L320-L326
        steps.append(
            {
                "settings": settings_record(logger),
                "skipped": returned is None,
                "request": request,
                "compress_calls": scripted.take_calls(),
                "lookup": sorted(cache),
                "diagnostics": recorder.diagnostics,
            }
        )
        recorder.diagnostics = []
    return steps


def run(call: InterceptionCall, _api_base: str) -> str:
    clock = Clock()
    real_time = time.time
    object.__setattr__(time, "time", clock)
    recorder = Recorder()
    logger = logging.getLogger("LiteLLM")
    previous_level = logger.level
    logger.addHandler(recorder)
    logger.setLevel(logging.DEBUG)
    scripted = Scripted()
    object.__setattr__(interception_handler, "compress", scripted.compress)
    steps = asyncio.run(run_steps(call, clock, recorder, scripted))
    logger.removeHandler(recorder)
    logger.setLevel(previous_level)
    object.__setattr__(time, "time", real_time)
    outcome: OutcomeValue = {"value": {"steps": steps}}
    return (
        TypeAdapter(dict[str, OutcomeValue])
        .dump_json({"compression_interception": outcome})
        .decode()
    )


class CompressionInterceptionRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    body: InterceptionCall = Field(alias="compression_interception")

    def serialize(self, api_base: str) -> str:
        return run(self.body, api_base)
