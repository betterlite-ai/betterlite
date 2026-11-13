from typing import Annotated, Literal
from unittest.mock import patch

from litellm.caching.redis_cache import (
    RedisCircuitBreaker,
    RedisCircuitBreakerOpenError,
    is_redis_timeout_failure,
)
from litellm.caching.redis_cache import (
    _BreakerAdmission as BreakerAdmission,  # pyright: ignore[reportPrivateUsage]
)
from litellm.caching.redis_cache import (
    _enter_circuit_breaker as enter_circuit_breaker,  # pyright: ignore[reportPrivateUsage]
)
from litellm.caching.redis_cache import (
    _exit_circuit_breaker as exit_circuit_breaker,  # pyright: ignore[reportPrivateUsage]
)
from litellm.caching.redis_cache import (
    _is_redis_health_failure as is_redis_health_failure,  # pyright: ignore[reportPrivateUsage]
)
from litellm.caching.redis_cache import (
    _RedisTimeoutLogThrottle as TimeoutLogThrottle,  # pyright: ignore[reportPrivateUsage]
)
from pydantic import Field, SerializeAsAny
from redis.exceptions import BusyLoadingError, ClusterDownError, ResponseError
from redis.exceptions import ConnectionError as RedisConnectionError
from redis.exceptions import TimeoutError as RedisTimeoutError
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

FailureClass = Literal[
    "connection",
    "timeout",
    "busy_loading",
    "cluster_down",
    "os",
    "builtin_timeout",
    "asynchronous_timeout",
    "command",
    "other",
    "circuit_open",
]

_ERROR_TYPES: dict[FailureClass, type[BaseException]] = {
    "connection": RedisConnectionError,
    "timeout": RedisTimeoutError,
    "busy_loading": BusyLoadingError,
    "cluster_down": ClusterDownError,
    "os": OSError,
    "builtin_timeout": TimeoutError,
    "asynchronous_timeout": TimeoutError,
    "command": ResponseError,
    "other": RuntimeError,
    "circuit_open": RedisCircuitBreakerOpenError,
}


class Settings(Strict):
    enabled: bool
    failure_threshold: int
    recovery_timeout_seconds: int
    timeout_min_duration_millis: int
    timeout_log_interval_millis: int


class AdmitStep(Strict):
    kind: Literal["admit"]
    now_millis: int
    operation: str


class FailureStep(Strict):
    kind: Literal["failure"]
    now_millis: int
    outer: FailureClass
    causes: list[FailureClass] = Field(default_factory=list)


class FinishStep(Strict):
    kind: Literal["finish"]
    admission: int


class ThrottleStep(Strict):
    kind: Literal["throttle"]
    now_millis: int


Step = Annotated[AdmitStep | FailureStep | FinishStep | ThrottleStep, Field(discriminator="kind")]


class Status(Strict):
    state: str
    generation: int
    failure_count: int
    hard_failure_count: int
    timeout_count: int


class AdmitOutput(Strict):
    kind: Literal["admit"] = "admit"
    admitted: bool
    error: str | None
    status: Status


class FailureOutput(Strict):
    kind: Literal["failure"] = "failure"
    health: bool
    timeout: bool
    opened: bool
    status: Status


class FinishOutput(Strict):
    kind: Literal["finish"] = "finish"
    recovered: bool
    status: Status


class ThrottleOutput(Strict):
    kind: Literal["throttle"] = "throttle"
    suppressed: int | None


StepOutput = Annotated[AdmitOutput | FailureOutput | FinishOutput | ThrottleOutput, Field(discriminator="kind")]


class Output(Strict):
    steps: list[SerializeAsAny[StepOutput]]


class LogClock:
    millis: int

    def __init__(self) -> None:
        self.millis = 0

    def seconds(self) -> float:
        return self.millis / 1000


def chained(outer: FailureClass, causes: list[FailureClass]) -> BaseException:
    names: list[FailureClass] = [outer, *causes]
    error: BaseException = _ERROR_TYPES[names[-1]]("probe")
    for name in reversed(names[:-1]):
        wrapper = _ERROR_TYPES[name]("probe")
        wrapper.__cause__ = error
        error = wrapper
    return error


def snapshot(breaker: RedisCircuitBreaker) -> Status:
    return Status(
        state=breaker._state,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        generation=breaker.generation,
        failure_count=breaker._failure_count,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        hard_failure_count=breaker._hard_failure_count,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        timeout_count=breaker._timeout_count,  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    )


class Harness:
    breaker: RedisCircuitBreaker
    clock: LogClock
    throttle: TimeoutLogThrottle
    admissions: dict[int, BreakerAdmission]

    def __init__(self, settings: Settings) -> None:
        self.breaker = RedisCircuitBreaker(
            failure_threshold=settings.failure_threshold,
            recovery_timeout=settings.recovery_timeout_seconds,
            enabled=settings.enabled,
            timeout_min_duration=settings.timeout_min_duration_millis / 1000,
        )
        self.clock = LogClock()
        self.throttle = TimeoutLogThrottle(
            settings.timeout_log_interval_millis / 1000, clock=self.clock.seconds
        )
        self.admissions = {}

    def step(self, index: int, step: Step) -> StepOutput:
        match step:
            case AdmitStep():
                return self.admit(index, step)
            case FailureStep():
                return self.failure(step)
            case FinishStep():
                return self.finish(step)
            case ThrottleStep():
                return self.throttled(step)

    def admit(self, index: int, step: AdmitStep) -> AdmitOutput:
        with patch("time.time", return_value=step.now_millis / 1000):
            try:
                admission = enter_circuit_breaker(self.breaker, step.operation)
            except RedisCircuitBreakerOpenError as error:
                return AdmitOutput(
                    admitted=False, error=str(error), status=snapshot(self.breaker)
                )
        self.admissions[index] = admission
        return AdmitOutput(admitted=True, error=None, status=snapshot(self.breaker))

    def failure(self, step: FailureStep) -> FailureOutput:
        error = chained(step.outer, step.causes)
        health = is_redis_health_failure(error)
        timeout = is_redis_timeout_failure(error)
        before = self.breaker._state  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        with patch("time.time", return_value=step.now_millis / 1000):
            if health:
                self.breaker.record_failure(is_timeout=timeout)
        after = self.breaker._state  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        return FailureOutput(
            health=health,
            timeout=timeout,
            opened=before != "open" and after == "open",
            status=snapshot(self.breaker),
        )

    def finish(self, step: FinishStep) -> FinishOutput:
        admission = self.admissions.get(step.admission)
        if admission is None:
            raise ProbeError(
                "invalid_request", "the admission index is not an admitted step"
            )
        before = self.breaker._state  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        exit_circuit_breaker(self.breaker, admission)
        after = self.breaker._state  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        return FinishOutput(
            recovered=before == "half_open" and after == "closed",
            status=snapshot(self.breaker),
        )

    def throttled(self, step: ThrottleStep) -> ThrottleOutput:
        self.clock.millis = step.now_millis
        return ThrottleOutput(suppressed=self.throttle.admit())


class RedisFailurePolicy(Operation):
    settings: Settings
    steps: Annotated[list[Step], Field(min_length=1)]

    @override
    def run(self) -> Output:
        harness = Harness(self.settings)
        steps = [
            harness.step(index, step) for index, step in enumerate(self.steps)
        ]
        return Output(steps=steps)
