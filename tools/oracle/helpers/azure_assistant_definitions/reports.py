from collections.abc import Generator
from contextlib import contextmanager

import httpx
import openai
from pydantic import JsonValue

from azure_assistant_definitions.sdk import SdkClient, Wire


@contextmanager
def reported(wire: Wire) -> Generator[None, None, None]:
    try:
        yield
    except openai.APIStatusError as error:
        wire.error = {"kind": "status", "status": error.status_code}
    except openai.OpenAIError:
        wire.error = {"kind": "configuration"}
    except ValueError:
        wire.error = {"kind": "value"}


def report(wire: Wire) -> JsonValue:
    values: dict[str, JsonValue] = {"requests": wire.seen}
    if wire.error is None:
        values["kind"] = wire.kind
        values["result"] = wire.result
    else:
        values["error"] = wire.error
    return values


def client_report(client: SdkClient, kind: str, source: str) -> JsonValue:
    timeout = client.timeout
    read_seconds: JsonValue = None
    match timeout:
        case httpx.Timeout():
            read = timeout.read
            if read is not None:
                read_seconds = float(read)
        case None:
            read_seconds = None
        case object():
            read_seconds = float(timeout)
    values: dict[str, JsonValue] = {
        "kind": kind,
        "source": source,
        "base_url": str(client.base_url).rstrip("/"),
        "max_retries": client.max_retries,
        "timeout": read_seconds,
    }
    return values
