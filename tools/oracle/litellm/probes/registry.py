import random
import sys
import time
from typing import Annotated
from warnings import warn

from pydantic import Field
from typing_extensions import override

from probes.protocol import SAMPLE_LIMIT, Clock, Operation, Strict, serve

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true


class EchoOutput(Strict):
    text: str
    clock: Clock
    samples: list[int]


class Echo(Operation):
    text: str
    draws: Annotated[int, Field(ge=0, lt=1 << 32)]
    warnings: list[str]

    @override
    def run(self) -> EchoOutput:
        for message in self.warnings:
            warn(message, stacklevel=1)
        clock = Clock(unix_millis=round(time.time() * 1000), monotonic_millis=round(time.monotonic() * 1000))
        # Echo reports the injected samples back, so it draws through the patched generator on purpose
        samples = [round(random.random() * SAMPLE_LIMIT) for _ in range(self.draws)]  # noqa: S311
        return EchoOutput(text=self.text, clock=clock, samples=samples)


OPERATIONS: dict[str, type[Operation]] = {"echo": Echo}

if __name__ == "__main__":
    serve(sys.stdin.buffer, sys.stdout, OPERATIONS)
