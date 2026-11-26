import asyncio

from pydantic import JsonValue

from .inputs import JSON, Call
from .keys import key_object
from .mapping import cache_key, mapping
from .session import session


async def answer(call: Call) -> JsonValue:
    match call["aspect"]:
        case "cache_key":
            return cache_key(call)
        case "mapping":
            return await mapping(call)
        case "key_object":
            return await key_object(call)
        case "session":
            return await session(call)


def run(input_value: Call) -> JsonValue:
    value = asyncio.run(answer(input_value))
    return JSON.validate_python(value)
