import asyncio
import json
from typing import Annotated, Literal, Protocol, cast

from litellm.caching.in_memory_cache import InMemoryCache
from pydantic import Field, SerializeAsAny
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

MAX_VALUE_BYTES = 1024 * 1024
PARSE_KEY = "probe-parse-value"


class CounterValue(Strict):
    counter: int


class SpendValue(Strict):
    spend: str


class PinFields(Strict):
    model_id: str


class PinValue(Strict):
    pin: PinFields


class SchedulerFields(Strict):
    priority: int
    request_id: str


class SchedulerValue(Strict):
    scheduler_tuple: SchedulerFields


class MembersValue(Strict):
    members: list[str]


RouteValue = CounterValue | SpendValue | PinValue | SchedulerValue | MembersValue


class SettingsInput(Strict):
    max_size_in_memory: int | None
    default_ttl_millis: int | None
    max_size_per_item: int | None


class Entry(Strict):
    key: str
    value: RouteValue


class SetStep(Strict):
    kind: Literal["set"]
    now_millis: int
    key: str
    value: RouteValue
    ttl_millis: int | None = None


class SetPipelineStep(Strict):
    kind: Literal["set_pipeline"]
    now_millis: int
    entries: list[Entry]
    ttl_millis: int | None = None


class SetAddStep(Strict):
    kind: Literal["set_add"]
    now_millis: int
    key: str
    values: list[str]
    ttl_millis: int | None = None


class GetStep(Strict):
    kind: Literal["get"]
    now_millis: int
    key: str


class BatchGetStep(Strict):
    kind: Literal["batch_get"]
    now_millis: int
    keys: list[str]


class IncrementStep(Strict):
    kind: Literal["increment"]
    now_millis: int
    key: str
    by: CounterValue | SpendValue
    ttl_millis: int | None = None


class CompareAndSetStep(Strict):
    kind: Literal["compare_and_set"]
    now_millis: int
    key: str
    expected: RouteValue | None = None
    value: RouteValue
    ttl_millis: int | None = None


class TextPayload(Strict):
    kind: Literal["text"]
    text: str


class PinPayload(Strict):
    kind: Literal["pin"]
    model_id_bytes: int


ParsePayload = Annotated[TextPayload | PinPayload, Field(discriminator="kind")]


class ParseStep(Strict):
    kind: Literal["parse"]
    payload: ParsePayload


Step = Annotated[
    SetStep
    | SetPipelineStep
    | SetAddStep
    | GetStep
    | BatchGetStep
    | IncrementStep
    | CompareAndSetStep
    | ParseStep,
    Field(discriminator="kind"),
]


class SetOutput(Strict):
    kind: Literal["set"] = "set"


class SetPipelineOutput(Strict):
    kind: Literal["set_pipeline"] = "set_pipeline"


class SetAddOutput(Strict):
    kind: Literal["set_add"] = "set_add"
    values: list[str]


class GetOutput(Strict):
    kind: Literal["get"] = "get"
    value: RouteValue | None


class BatchGetOutput(Strict):
    kind: Literal["batch_get"] = "batch_get"
    values: list[RouteValue | None]


class IncrementOutput(Strict):
    kind: Literal["increment"] = "increment"
    value: RouteValue


class CompareAndSetOutput(Strict):
    kind: Literal["compare_and_set"] = "compare_and_set"
    swapped: bool
    current: RouteValue | None


class CounterSummary(Strict):
    kind: Literal["counter"] = "counter"
    value: int


class SpendSummary(Strict):
    kind: Literal["spend"] = "spend"
    amount: str


class PinSummary(Strict):
    kind: Literal["pin"] = "pin"
    model_id_bytes: int


class SchedulerSummary(Strict):
    kind: Literal["scheduler_tuple"] = "scheduler_tuple"
    priority: int
    request_id_bytes: int


class MembersSummary(Strict):
    kind: Literal["members"] = "members"
    members: list[str]


ValueSummary = CounterSummary | SpendSummary | PinSummary | SchedulerSummary | MembersSummary


class ValidParse(Strict):
    kind: Literal["valid"] = "valid"
    value: ValueSummary


class InvalidParse(Strict):
    kind: Literal["invalid"] = "invalid"
    bytes: int
    limit: int


ParseResult = ValidParse | InvalidParse


class ParseOutput(Strict):
    kind: Literal["parse"] = "parse"
    result: ParseResult


StepOutput = Annotated[
    SetOutput
    | SetPipelineOutput
    | SetAddOutput
    | GetOutput
    | BatchGetOutput
    | IncrementOutput
    | CompareAndSetOutput
    | ParseOutput,
    Field(discriminator="kind"),
]


class Output(Strict):
    steps: list[SerializeAsAny[StepOutput]]


class MemoryCache(Protocol):
    cache_dict: dict[str, object]

    async def async_set_cache(self, key: str, value: object, *, ttl: float | None = None) -> None: ...

    async def async_set_cache_pipeline(
        self, cache_list: list[tuple[str, object]], *, ttl: float | None = None
    ) -> None: ...

    async def async_set_cache_sadd(self, key: str, value: list[str], ttl: float | None) -> list[str]: ...

    def batch_get_cache(self, keys: list[str]) -> list[object]: ...

    def get_cache(self, key: str) -> object: ...

    def set_cache(self, key: str, value: object, *, ttl: float | None = None) -> None: ...

    def increment_cache(self, key: str, value: float, *, ttl: float | None = None) -> float: ...


class Clock:
    now: float

    def __init__(self) -> None:
        self.now = 0.0

    def seconds(self) -> float:
        return self.now


def stored_value(value: RouteValue) -> object:
    if isinstance(value, CounterValue):
        return value.counter
    if isinstance(value, SpendValue):
        return float(value.spend)
    if isinstance(value, PinValue):
        return {"model_id": value.pin.model_id}
    if isinstance(value, SchedulerValue):
        return (value.scheduler_tuple.priority, value.scheduler_tuple.request_id)
    return set(value.members)


def scheduler_output(value: tuple[int, str]) -> SchedulerValue:
    priority, request_id = value
    return SchedulerValue(scheduler_tuple=SchedulerFields(priority=priority, request_id=request_id))


def pin_output(value: dict[str, object]) -> PinValue:
    model_id = value.get("model_id")
    if not isinstance(model_id, str):
        raise ProbeError("invalid_request", "a cached pin must hold a string model_id")
    return PinValue(pin=PinFields(model_id=model_id))


def output_value(value: object) -> RouteValue | None:
    if value is None:
        return None
    if isinstance(value, bool):
        raise ProbeError("invalid_request", "a cached boolean is not a route value")
    if isinstance(value, int):
        return CounterValue(counter=value)
    if isinstance(value, float):
        return SpendValue(spend=repr(value))
    if isinstance(value, set):
        return MembersValue(members=sorted(cast("set[str]", value)))
    if isinstance(value, tuple):
        return scheduler_output(cast("tuple[int, str]", value))
    if isinstance(value, dict):
        return pin_output(cast("dict[str, object]", value))
    raise ProbeError("invalid_request", f"a cached value of type {type(value).__name__} is not a route value")


def required_output(value: object) -> RouteValue:
    output = output_value(value)
    if output is None:
        raise ProbeError("invalid_request", "an increment returned no value")
    return output


def read(cache: MemoryCache, key: str) -> object:
    values = cache.batch_get_cache([key])
    if not values:
        raise ProbeError("invalid_request", "a batch read returned no value")
    return values[0]


def increment_amount(by: CounterValue | SpendValue) -> int | float:
    if isinstance(by, CounterValue):
        return by.counter
    return float(by.spend)


def seconds_from_millis(millis: int | None) -> float | None:
    if millis is None:
        return None
    return millis / 1000


def set_step(cache: MemoryCache, clock: Clock, step: SetStep) -> StepOutput:
    clock.now = step.now_millis / 1000
    asyncio.run(cache.async_set_cache(step.key, stored_value(step.value), ttl=seconds_from_millis(step.ttl_millis)))
    return SetOutput()


def set_pipeline_step(cache: MemoryCache, clock: Clock, step: SetPipelineStep) -> StepOutput:
    clock.now = step.now_millis / 1000
    entries = [(entry.key, stored_value(entry.value)) for entry in step.entries]
    asyncio.run(cache.async_set_cache_pipeline(entries, ttl=seconds_from_millis(step.ttl_millis)))
    return SetPipelineOutput()


def set_add_step(cache: MemoryCache, clock: Clock, step: SetAddStep) -> StepOutput:
    clock.now = step.now_millis / 1000
    values = asyncio.run(cache.async_set_cache_sadd(step.key, step.values, seconds_from_millis(step.ttl_millis)))
    return SetAddOutput(values=values)


def get_step(cache: MemoryCache, clock: Clock, step: GetStep) -> StepOutput:
    clock.now = step.now_millis / 1000
    return GetOutput(value=output_value(read(cache, step.key)))


def batch_get_step(cache: MemoryCache, clock: Clock, step: BatchGetStep) -> StepOutput:
    clock.now = step.now_millis / 1000
    return BatchGetOutput(values=[output_value(value) for value in cache.batch_get_cache(step.keys)])


def increment_step(cache: MemoryCache, clock: Clock, step: IncrementStep) -> StepOutput:
    clock.now = step.now_millis / 1000
    result = cache.increment_cache(step.key, increment_amount(step.by), ttl=seconds_from_millis(step.ttl_millis))
    return IncrementOutput(value=required_output(result))


def compare_and_set_step(cache: MemoryCache, clock: Clock, step: CompareAndSetStep) -> StepOutput:
    clock.now = step.now_millis / 1000
    current = read(cache, step.key)
    expected = None if step.expected is None else stored_value(step.expected)
    if current != expected:
        return CompareAndSetOutput(swapped=False, current=output_value(current))
    candidate = stored_value(step.value)
    cache.set_cache(step.key, candidate, ttl=seconds_from_millis(step.ttl_millis))
    return CompareAndSetOutput(swapped=True, current=output_value(candidate))


def payload_text(payload: ParsePayload) -> str:
    if isinstance(payload, TextPayload):
        return payload.text
    if payload.model_id_bytes > MAX_VALUE_BYTES + 1:
        raise ProbeError("invalid_request", "the pin model id length is past the parser bound")
    return json.dumps({"pin": {"model_id": "a" * payload.model_id_bytes}})


def string_list(values: list[object]) -> list[str]:
    if not all(isinstance(value, str) for value in values):
        raise ProbeError("invalid_request", "a parsed member set must hold strings")
    return [cast("str", value) for value in values]


def pin_summary(fields: dict[str, object]) -> PinSummary | None:
    pin = fields.get("pin")
    if not isinstance(pin, dict):
        return None
    model_id = cast("dict[str, object]", pin).get("model_id")
    if not isinstance(model_id, str):
        return None
    return PinSummary(model_id_bytes=len(model_id.encode("utf-8")))


def scheduler_summary(fields: dict[str, object]) -> SchedulerSummary | None:
    scheduler = fields.get("scheduler_tuple")
    if not isinstance(scheduler, dict):
        return None
    schedule = cast("dict[str, object]", scheduler)
    priority = schedule.get("priority")
    request_id = schedule.get("request_id")
    if not isinstance(priority, int) or not isinstance(request_id, str):
        return None
    return SchedulerSummary(priority=priority, request_id_bytes=len(request_id.encode("utf-8")))


def scalar_summary(fields: dict[str, object]) -> ValueSummary:
    counter = fields.get("counter")
    if isinstance(counter, int) and not isinstance(counter, bool):
        return CounterSummary(value=counter)
    spend = fields.get("spend")
    if isinstance(spend, str):
        return SpendSummary(amount=spend)
    members = fields.get("members")
    if isinstance(members, list):
        return MembersSummary(members=string_list(cast("list[object]", members)))
    raise ProbeError("invalid_request", "the parsed value is not a route value")


def summary(value: object) -> ValueSummary:
    if not isinstance(value, dict):
        raise ProbeError("invalid_request", "the parsed value is not a route value")
    fields = cast("dict[str, object]", value)
    return pin_summary(fields) or scheduler_summary(fields) or scalar_summary(fields)


def parse_step(cache: MemoryCache, step: ParseStep) -> StepOutput:
    cache.cache_dict[PARSE_KEY] = payload_text(step.payload)
    return ParseOutput(result=ValidParse(value=summary(cache.get_cache(PARSE_KEY))))


def execute(cache: MemoryCache, clock: Clock, step: Step) -> StepOutput:
    match step:
        case SetStep():
            output = set_step(cache, clock, step)
        case SetPipelineStep():
            output = set_pipeline_step(cache, clock, step)
        case SetAddStep():
            output = set_add_step(cache, clock, step)
        case GetStep():
            output = get_step(cache, clock, step)
        case BatchGetStep():
            output = batch_get_step(cache, clock, step)
        case IncrementStep():
            output = increment_step(cache, clock, step)
        case CompareAndSetStep():
            output = compare_and_set_step(cache, clock, step)
        case ParseStep():
            output = parse_step(cache, step)
    return output


def whole_seconds(value: int | None) -> int | None:
    if value is None:
        return None
    if value % 1000 != 0:
        raise ProbeError("invalid_request", "default_ttl_millis must be whole seconds")
    return value // 1000


class RoutingValueStore(Operation):
    settings: SettingsInput
    steps: list[Step]

    @override
    def run(self) -> Output:
        clock = Clock()
        cache = cast(
            "MemoryCache",
            InMemoryCache(
                max_size_in_memory=self.settings.max_size_in_memory,
                default_ttl=whole_seconds(self.settings.default_ttl_millis),
                max_size_per_item=self.settings.max_size_per_item,
                clock=clock.seconds,
            ),
        )
        return Output(steps=[execute(cache, clock, step) for step in self.steps])
