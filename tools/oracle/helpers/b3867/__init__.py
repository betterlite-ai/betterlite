import asyncio

from pydantic import JsonValue

from .service import dispatch
from .types import Input


def run(input_value: Input) -> JsonValue:
    return asyncio.run(dispatch(input_value))
