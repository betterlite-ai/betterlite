import random
import time
from io import BytesIO, StringIO

import pytest

from probes.protocol import MAX_MESSAGE_BYTES, serve
from probes.registry import OPERATIONS

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true

CLOCK = '"clock":{"unix_millis":1700000000123,"monotonic_millis":42000}'
OPERATION = '"operation":{"echo":{"text":"ping","draws":2,"warnings":["stale health snapshot"]}}'
REQUEST = "{" + CLOCK + ',"samples":[0,9007199254740991],' + OPERATION + "}"
REPLY = (
    '{"outcome":"ok","output":{"echo":{"text":"ping",'
    + CLOCK
    + ',"samples":[0,9007199254740991]}},"warnings":["stale health snapshot"]}'
)


def run(request: str) -> str:
    reply = StringIO()
    serve(BytesIO(request.encode()), reply, OPERATIONS)
    return reply.getvalue()


def test_echo_sees_the_injected_clock_and_samples_then_restores_them() -> None:
    originals = (time.time, random.random, random.choice)
    assert run(REQUEST) == REPLY
    assert (time.time, random.random, random.choice) == originals


@pytest.mark.parametrize(
    ("request_text", "reply_start"),
    [
        (REQUEST.replace('"echo"', '"shuffle"'), '{"outcome":"error","kind":"unknown_operation","message":"shuffle"}'),
        (REQUEST.replace('"draws":2', '"draws":3'), '{"outcome":"error","kind":"samples_exhausted"'),
        (REQUEST.replace("9007199254740991", "9007199254740992"), '{"outcome":"error","kind":"invalid_request"'),
        (REQUEST.replace('"draws":2', '"draws":true'), '{"outcome":"error","kind":"invalid_request"'),
        (REQUEST.replace('"draws":2', '"draws":2.0'), '{"outcome":"error","kind":"invalid_request"'),
        (REQUEST.replace('"text"', '"model":"x","text"'), '{"outcome":"error","kind":"invalid_request"'),
        (REQUEST.replace('"samples"', '"seed":4,"samples"'), '{"outcome":"error","kind":"invalid_request"'),
        (REQUEST + " " * MAX_MESSAGE_BYTES, '{"outcome":"error","kind":"invalid_request"'),
    ],
    ids=["unknown", "exhausted", "sample_limit", "bool", "float", "input_field", "request_field", "oversized"],
)
def test_requests_outside_the_schema_get_typed_errors(request_text: str, reply_start: str) -> None:
    assert run(request_text).startswith(reply_start)
