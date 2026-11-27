from datetime import UTC, datetime, tzinfo
from typing import ClassVar, Literal, NotRequired, Self, TypeAlias, TypedDict

from litellm.proxy.lens import inference
from litellm.proxy.lens.models import Job, Lens, ModelRequest, Worker
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

KEY = "a" * 64

# LiteLLM's Database protocol takes object arguments, so a narrower type breaks it
SqlArgument: TypeAlias = object


class DeploymentParams(TypedDict, total=False):
    model: str
    input_cost_per_token: float | None
    output_cost_per_token: float | None


class DeploymentCall(TypedDict):
    litellm_params: DeploymentParams


class UsageCall(TypedDict):
    prompt_tokens: int
    completion_tokens: int


class Call(TypedDict):
    operation: Literal["quote", "reserve", "settle", "charge", "request_body"]
    prompt: NotRequired[str]
    deployments: NotRequired[list[DeploymentCall]]
    model_info: NotRequired[dict[str, dict[str, JsonValue]]]
    lens: NotRequired[dict[str, JsonValue]]
    job_id: NotRequired[str]
    worker_id: NotRequired[str]
    now: NotRequired[list[str]]
    team_id: NotRequired[str | None]
    cost: NotRequired[float | None]
    job: NotRequired[dict[str, JsonValue]]
    estimate: NotRequired[float]
    response: NotRequired[dict[str, JsonValue]]


class StopEarlyError(Exception):
    pass


class Clock(datetime):
    readings: ClassVar[tuple[datetime, ...]] = ()
    position: ClassVar[int] = 0

    @classmethod
    @override
    def now(cls, tz: tzinfo | None = None) -> Self:
        moment = Clock.readings[Clock.position]
        Clock.position += 1
        zone = tz if tz is not None else UTC
        return cls(
            moment.year,
            moment.month,
            moment.day,
            moment.hour,
            moment.minute,
            moment.second,
            moment.microsecond,
            tzinfo=zone,
        )


def dump(lens: Lens) -> JsonValue:
    text = lens.model_dump_json()
    return TypeAdapter(JsonValue).validate_json(text)


class Store:
    def __init__(self, lens: Lens) -> None:
        self.lens = lens
        self.version = lens.version

    async def query_raw(self, query: str, *args: SqlArgument) -> list[dict[str, JsonValue]]:
        if query.startswith("WITH previous"):
            if int(str(args[2])) != self.version:
                return [{"data": 0}]
            self.lens = Lens.model_validate_json(str(args[0]))
            self.version += 1
            return [{"data": 1}]
        if len(args) == 0 or str(args[0]) != self.lens.id:
            return []
        return [{"data": dump(self.lens)}]

    async def execute_raw(self, query: str, *args: SqlArgument) -> int:
        message = f"the lens fixture cannot execute {query} with {len(args)} arguments"
        raise NotImplementedError(message)


def prompt_of(call: Call) -> str:
    prompt = call.get("prompt")
    if prompt is None:
        raise ValueError("a call needs prompt")
    return prompt


def deployment_calls(call: Call) -> list[DeploymentCall]:
    deployments = call.get("deployments")
    if deployments is None:
        raise ValueError("a call needs deployments")
    return deployments


def deployments_of(call: Call) -> tuple[inference.Deployment, ...]:
    deployments = deployment_calls(call)
    return tuple(inference.Deployment.model_validate(deployment) for deployment in deployments)


def model_info_of(call: Call) -> dict[str, dict[str, JsonValue]]:
    entries = call.get("model_info")
    if entries is None:
        return {}
    return entries


def usage_of(call: Call) -> UsageCall:
    payload = call.get("response")
    if payload is None:
        return UsageCall(prompt_tokens=0, completion_tokens=0)
    usage = payload.get("usage")
    if usage is None:
        raise ValueError("a charge needs the response usage")
    return TypeAdapter(UsageCall).validate_python(usage)


def readings_of(call: Call) -> tuple[datetime, ...]:
    readings = call.get("now")
    if readings is None:
        raise ValueError("an analysis call needs now")
    return tuple(datetime.fromisoformat(reading) for reading in readings)


def lens_of(call: Call) -> Lens:
    payload = call.get("lens")
    if payload is None:
        raise ValueError("an analysis call needs lens")
    return Lens.model_validate(payload)


def job_of(call: Call, lens: Lens) -> Job:
    payload = call.get("job")
    if payload is not None:
        return Job.model_validate(payload)
    job_id = call.get("job_id")
    if job_id is None:
        raise ValueError("an analysis call needs job_id")
    for job in lens.jobs:
        if job.id == job_id:
            return job
    raise ValueError("the lens has no such job")


def worker_of(call: Call) -> Worker:
    worker_id = call.get("worker_id")
    if worker_id is None:
        raise ValueError("an analysis call needs worker_id")
    scope: dict[str, JsonValue] = {}
    return Worker(
        id=worker_id,
        name="worker",
        scope=scope,
        last_seen="2026-01-01T00:00:00Z",
        analysis_key_id=KEY,
    )


def body_of(call: Call) -> ModelRequest:
    payload: dict[str, JsonValue] = {"prompt": prompt_of(call), "purpose": "cluster"}
    return ModelRequest.model_validate(payload)


def lens_state(store: Store) -> JsonValue:
    lens = store.lens
    cost = lens.jobs[0].cost if lens.jobs else 0.0
    return {
        "version": store.version,
        "spent": lens.spent,
        "budget_month": lens.budget_month,
        "cost": cost,
    }
