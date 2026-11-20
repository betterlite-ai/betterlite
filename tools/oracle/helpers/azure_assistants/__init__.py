import asyncio
import os
from types import TracebackType
from typing import Self

import httpx
import litellm
import openai
from openai import AsyncAzureOpenAI, AzureOpenAI
from pydantic import JsonValue

from azure_assistant_definitions.sdk import SdkClient, record
from azure_assistants.calls import (
    ENDPOINT,
    KEY,
    NO_REPLY,
    TIMEOUT,
    VERSION,
    Input,
    Reply,
    RunStep,
    dispatched,
)


# The SDK client in place of the network: the case env, queued replies and recorded requests.
class Session:
    def __init__(self, step: RunStep) -> None:
        self.replies: list[Reply] = []
        if "replies" in step:
            self.replies = step["replies"]
        self.environment: dict[str, str] = {}
        if "environment" in step:
            self.environment = step["environment"]
        self.seen: list[JsonValue] = []

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.seen.append(record(request))
        reply = NO_REPLY
        if self.replies:
            reply = self.replies.pop(0)
        return httpx.Response(reply["status"], content=reply["body"], headers=reply["headers"])

    def __enter__(self) -> Self:
        self.saved = dict(os.environ)
        os.environ.clear()
        os.environ.update(self.environment)
        self.sync_client = httpx.Client(transport=httpx.MockTransport(self))
        self.async_client = httpx.AsyncClient(transport=httpx.MockTransport(self))
        litellm.client_session = self.sync_client
        litellm.aclient_session = self.async_client
        return self

    def __exit__(
        self,
        _kind: type[BaseException] | None,
        _value: BaseException | None,
        _trace: TracebackType | None,
    ) -> None:
        litellm.client_session = None
        litellm.aclient_session = None
        self.sync_client.close()
        asyncio.run(self.async_client.aclose())
        os.environ.clear()
        os.environ.update(self.saved)


# A client the caller supplied, matching the one the Rust helper builds for the same step.
def supplied_client(step: RunStep, session: Session) -> SdkClient | None:
    supplied = step.get("supplied")
    if supplied is None:
        return None
    transport = httpx.MockTransport(session)
    if supplied["kind"] == "async":
        return AsyncAzureOpenAI(
            api_key=KEY,
            azure_endpoint=ENDPOINT,
            api_version=VERSION,
            timeout=TIMEOUT,
            http_client=httpx.AsyncClient(transport=transport),
        )
    return AzureOpenAI(
        api_key=KEY,
        azure_endpoint=ENDPOINT,
        api_version=VERSION,
        timeout=TIMEOUT,
        http_client=httpx.Client(transport=transport),
    )


def step_outcome(step: RunStep) -> JsonValue:
    calls: list[JsonValue] = []
    with Session(step) as session:
        failure: JsonValue = None
        produced: dict[str, JsonValue] = {}
        try:
            produced = dispatched(step, supplied_client(step, session), calls)
        except openai.APIStatusError as error:
            failure = {"kind": "status", "status": error.status_code}
        except openai.OpenAIError:
            failure = {"kind": "configuration"}
        except ValueError:
            failure = {"kind": "value"}
        outcome: dict[str, JsonValue] = {"requests": session.seen, "calls": calls}
        if failure is not None:
            outcome["error"] = failure
        else:
            outcome.update(produced)
        return outcome


def steps_of(steps: list[RunStep]) -> JsonValue:
    outcomes: list[JsonValue] = [step_outcome(step) for step in steps]
    return {"steps": outcomes}


def steps_of_variant(argument: Input) -> list[RunStep]:
    steps = argument.get("run_poll") or argument.get("run_stream")
    steps = steps or argument.get("run_instructions_tools")
    steps = steps or argument.get("run_failure") or argument.get("run_dispatch")
    steps = steps or argument.get("instructions_tools_overrides")
    steps = steps or argument.get("stream_event_handler")
    steps = steps or argument.get("assistants_beta_default")
    if steps is None:
        raise ValueError("expected one azure assistants variant")
    return steps


def run(argument: Input) -> JsonValue:
    steps = steps_of_variant(argument)
    return steps_of(steps)
