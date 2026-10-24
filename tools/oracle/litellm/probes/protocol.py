import random
import time
import warnings
from collections.abc import Generator, Mapping
from contextlib import contextmanager
from typing import Annotated, BinaryIO, ClassVar, Literal, TextIO

from pydantic import BaseModel, ConfigDict, Field, JsonValue, SerializeAsAny, ValidationError
from typing_extensions import override

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true

MAX_MESSAGE_BYTES = 1 << 20
SAMPLE_LIMIT = 1 << 53

Failure = Literal["invalid_request", "unknown_operation", "samples_exhausted"]
Millis = Annotated[int, Field(ge=0, lt=1 << 64)]


class ProbeError(Exception):
    def __init__(self, kind: Failure, message: str) -> None:
        super().__init__(message)
        self.kind: Failure = kind


class Strict(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="forbid", strict=True, frozen=True)


class Operation(Strict):
    def run(self) -> Strict:
        raise NotImplementedError


class Clock(Strict):
    unix_millis: Millis
    monotonic_millis: Millis

    def wall(self) -> float:
        return self.unix_millis / 1000

    def monotonic(self) -> float:
        return self.monotonic_millis / 1000


class Request(Strict):
    clock: Clock
    samples: list[Annotated[int, Field(ge=0, lt=SAMPLE_LIMIT)]]
    operation: Annotated[dict[str, JsonValue], Field(min_length=1, max_length=1)]


class Reply(Strict):
    outcome: Literal["ok"] = "ok"
    output: dict[str, SerializeAsAny[Strict]]
    warnings: list[str]


class ErrorReply(Strict):
    outcome: Literal["error"] = "error"
    kind: Failure
    message: str


class InjectedRandom(random.Random):
    def __init__(self, samples: list[int]) -> None:
        super().__init__(0)
        self.remaining: list[int] = samples[::-1]

    # Random routes choice, shuffle and sample through random() when a subclass overrides only random()
    @override
    def random(self) -> float:
        if not self.remaining:
            raise ProbeError("samples_exhausted", "the request has no sample left for this draw")
        # a 53-bit integer over 2**53 is exactly the float the stock random() would return
        return self.remaining.pop() / SAMPLE_LIMIT


@contextmanager
def injected(clock: Clock, generator: InjectedRandom) -> Generator[None, None, None]:
    saved = (time.time, time.monotonic, random.random, random.choices, random.choice, random.shuffle)
    time.time, time.monotonic = clock.wall, clock.monotonic
    random.random, random.choices = generator.random, generator.choices
    random.choice, random.shuffle = generator.choice, generator.shuffle
    try:
        yield
    finally:
        time.time, time.monotonic, random.random, random.choices, random.choice, random.shuffle = saved


def answer(data: bytes, operations: Mapping[str, type[Operation]]) -> Reply:
    if len(data) > MAX_MESSAGE_BYTES:
        raise ProbeError("invalid_request", "the request exceeds 1 MiB")
    try:
        request = Request.model_validate_json(data)
        ((name, arguments),) = request.operation.items()
        operation_type = operations.get(name)
        if operation_type is None:
            raise ProbeError("unknown_operation", name)
        operation = operation_type.model_validate(arguments)
    except ValidationError as error:
        raise ProbeError("invalid_request", str(error)) from error
    with injected(request.clock, InjectedRandom(request.samples)), warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        output = operation.run()
    return Reply(output={name: output}, warnings=[str(warning.message) for warning in caught])


def serve(stdin: BinaryIO, stdout: TextIO, operations: Mapping[str, type[Operation]]) -> None:
    data = stdin.read(MAX_MESSAGE_BYTES + 1)
    try:
        reply: Reply | ErrorReply = answer(data, operations)
    except ProbeError as error:
        reply = ErrorReply(kind=error.kind, message=str(error))
    _ = stdout.write(reply.model_dump_json())
