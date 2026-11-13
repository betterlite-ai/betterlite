import os
from unittest import mock

from pydantic import JsonValue as Json

from .answers import answer
from .contracts import Input


def run(input_value: Input) -> Json:
    with mock.patch.dict(os.environ, input_value["environment"], clear=True):
        return {"calls": [answer(call) for call in input_value["calls"]]}
