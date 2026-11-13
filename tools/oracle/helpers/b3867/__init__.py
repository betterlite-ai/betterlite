import asyncio

from pydantic import JsonValue

from .service import dispatch
from .types import Input


def run(input_value: Input) -> JsonValue:
    loop = asyncio.new_event_loop()
    try:
        return loop.run_until_complete(dispatch(input_value))
    finally:
        loop.close()
