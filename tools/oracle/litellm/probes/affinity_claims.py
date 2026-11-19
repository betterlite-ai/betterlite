import asyncio
import warnings
from collections.abc import Awaitable, Callable
from dataclasses import dataclass
from typing import Annotated, Final, Literal

from litellm.caching.affinity_cache import claim_affinity_pin, set_local_affinity_pin
from litellm.caching.dual_cache import DualCache
from litellm.caching.in_memory_cache import InMemoryCache
from litellm.caching.redis_cache import RedisCache
from litellm.router_utils.pre_call_checks.deployment_affinity_check import DeploymentAffinityCheck
from pydantic import Field, TypeAdapter, ValidationError
from redis.exceptions import ConnectionError as RedisConnectionError
from typing_extensions import TypedDict, override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

PARSE_KEY: Final = "affinity-parse-key"
PROBE_TTL: Final = 60
ELIGIBLE_ARGUMENT: Final = 2
MAX_PARSE_PAD: Final = 2_097_152


class PinMap(TypedDict, total=False):
    model_id: str


PIN_MAP: Final = TypeAdapter(PinMap)
ELIGIBLE: Final = TypeAdapter(list[dict[str, str]])


class PinField(Strict):
    name: str
    value: str


class KeyRequest(Strict):
    kind: Literal["affinity", "session"]
    model_group: str
    session_id: str | None = None
    user_key: str | None = None


class KeyStep(Strict):
    kind: Literal["key"] = "key"
    request: KeyRequest


class ClaimStep(Strict):
    kind: Literal["claim"] = "claim"
    key: str
    pin: list[PinField]
    ttl_seconds: int
    down: bool = False
    eligible: list[list[PinField]] | None = None


class SeedStep(Strict):
    kind: Literal["seed"] = "seed"
    target: Literal["local", "shared"]
    key: str
    raw: str
    ttl_seconds: int


class ParseStep(Strict):
    kind: Literal["parse"] = "parse"
    pad_len: int


Step = Annotated[
    KeyStep | ClaimStep | SeedStep | ParseStep,
    Field(discriminator="kind"),
]


class KeyOutput(Strict):
    kind: Literal["key"] = "key"
    key: str | None


class ClaimOutput(Strict):
    kind: Literal["claim"] = "claim"
    pin_kind: str
    model_id: str | None
    text: str | None
    fallback: str | None


class SeedOutput(Strict):
    kind: Literal["seed"] = "seed"


class ParseOutput(Strict):
    kind: Literal["parse"] = "parse"
    shape: str
    byte_count: int
    reason: str | None


Observation = Annotated[
    KeyOutput | ClaimOutput | SeedOutput | ParseOutput,
    Field(discriminator="kind"),
]


class Output(Strict):
    steps: list[Observation]


@dataclass
class FakeStore(RedisCache):
    values: dict[str, str]
    down: bool
    failed: bool

    @override
    def async_register_script(self, _script: str) -> Callable[..., Awaitable[object]]:
        return self.run_claim

    async def run_claim(self, keys: tuple[str, ...], args: tuple[str | int, ...]) -> object:
        if self.down:
            self.failed = True
            raise RedisConnectionError("connection down")
        key = keys[0]
        pin_json = str(args[0])
        current = self.values.get(key)
        if current is None:
            self.values[key] = pin_json
            return pin_json
        eligible_text = str(args[ELIGIBLE_ARGUMENT]) if len(args) > ELIGIBLE_ARGUMENT else None
        if eligible_text is not None and self.matches(current, eligible_text):
            return current
        if eligible_text is not None:
            self.values[key] = pin_json
            return pin_json
        return current

    def matches(self, current: str, eligible_text: str) -> bool:
        try:
            stored = PIN_MAP.validate_json(current)
        except ValidationError:
            return False
        candidates = ELIGIBLE.validate_json(eligible_text)
        return any(stored == candidate for candidate in candidates)


@dataclass
class Context:
    cache: DualCache
    store: FakeStore
    shared: bool


def pin_value(fields: list[PinField]) -> dict[str, str]:
    return {field.name: field.value for field in fields}


def claim_summary(winner: object) -> tuple[str, str | None, str | None]:
    if isinstance(winner, str):
        return ("text", None, winner)
    if isinstance(winner, dict):
        fields = PIN_MAP.validate_python(winner)
        return ("pin", fields.get("model_id"), None)
    return ("other", None, None)


def key_step(step: KeyStep) -> KeyOutput:
    request = step.request
    if request.kind == "affinity":
        if request.user_key is None:
            raise ProbeError("invalid_request", "an affinity key needs a user key")
        return KeyOutput(key=DeploymentAffinityCheck.get_affinity_cache_key(request.model_group, request.user_key))
    if request.session_id is None:
        raise ProbeError("invalid_request", "a session key needs a session id")
    return KeyOutput(
        key=DeploymentAffinityCheck.get_session_affinity_cache_key(
            request.model_group,
            request.session_id,
            request.user_key,
        )
    )


async def claim_step(step: ClaimStep, context: Context) -> ClaimOutput:
    context.store.down = step.down
    context.store.failed = False
    eligible = None if step.eligible is None else tuple(pin_value(entry) for entry in step.eligible)
    winner = await claim_affinity_pin(
        context.cache,
        step.key,
        pin_value(step.pin),
        step.ttl_seconds,
        eligible_values=eligible,
    )
    fallback = "unavailable" if context.store.failed else None
    pin_kind, model_id, text = claim_summary(winner)
    return ClaimOutput(pin_kind=pin_kind, model_id=model_id, text=text, fallback=fallback)


def seed_step(step: SeedStep, context: Context) -> SeedOutput:
    if step.target == "local":
        set_local_affinity_pin(context.cache, step.key, step.raw, step.ttl_seconds)
        return SeedOutput()
    if not context.shared:
        raise ProbeError("invalid_request", "the shared store is not configured")
    context.store.values[step.key] = step.raw
    return SeedOutput()


async def parse_step(step: ParseStep) -> ParseOutput:
    if step.pad_len < 0 or step.pad_len > MAX_PARSE_PAD:
        raise ProbeError("invalid_request", "the parse pad is past the 2097152-byte build limit")
    text = f'{{"model_id":"{"a" * step.pad_len}"}}'
    store = FakeStore(values={PARSE_KEY: text}, down=False, failed=False)
    cache = DualCache(in_memory_cache=InMemoryCache(), redis_cache=store)
    winner = await claim_affinity_pin(cache, PARSE_KEY, {"model_id": "__probe__"}, PROBE_TTL)
    if isinstance(winner, str):
        return ParseOutput(shape="text", byte_count=len(winner.encode("utf-8")), reason=None)
    if isinstance(winner, dict):
        fields = PIN_MAP.validate_python(winner)
        model_id = fields.get("model_id")
        return ParseOutput(
            shape="pin",
            byte_count=0 if model_id is None else len(model_id.encode("utf-8")),
            reason=None,
        )
    return ParseOutput(shape="other", byte_count=0, reason=None)


async def apply_step(step: Step, context: Context) -> Observation:
    if isinstance(step, KeyStep):
        return key_step(step)
    if isinstance(step, SeedStep):
        return seed_step(step, context)
    if isinstance(step, ClaimStep):
        return await claim_step(step, context)
    return await parse_step(step)


async def run_scenario(operation: AffinityClaims) -> Output:
    store = FakeStore(values={}, down=False, failed=False)
    cache = DualCache(in_memory_cache=InMemoryCache(), redis_cache=store if operation.shared else None)
    context = Context(cache=cache, store=store, shared=operation.shared)
    return Output(steps=[await apply_step(step, context) for step in operation.steps])


class AffinityClaims(Operation):
    shared: bool
    steps: list[Step]

    @override
    def run(self) -> Strict:
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            return asyncio.run(run_scenario(self))
