import asyncio
from typing import Final, Literal, NotRequired, TypedDict

import litellm
import openai
from openai_chat import Failure, project_failure
from pydantic import BaseModel, ConfigDict, JsonValue as Json, TypeAdapter

JSON_DICT: Final[TypeAdapter[dict[str, Json]]] = TypeAdapter(dict[str, Json])


class Settings(TypedDict):
    api_key: str | None
    api_version: str | None


class Run(TypedDict):
    mode: Literal["sync", "async"]
    operation: Literal["append", "list"]
    provider: str
    thread_id: str
    role: str | None
    content: Json
    attachments: Json
    metadata: Json
    params: NotRequired[dict[str, Json]]
    settings: Settings
    drop_params: bool


class AssistantMessagesCall(TypedDict):
    runs: list[Run]


class RunOutcome(TypedDict):
    value: NotRequired[Json]
    failure: NotRequired[Failure]


class Outcome(TypedDict):
    assistant_messages: dict[Literal["value"], dict[Literal["runs"], list[RunOutcome]]]


class AssistantMessagesRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    assistant_messages: AssistantMessagesCall

    def serialize(self, api_base: str) -> str:
        runs = [run_outcome(run, api_base) for run in self.assistant_messages["runs"]]
        outcome: Outcome = {"assistant_messages": {"value": {"runs": runs}}}
        return TypeAdapter(Outcome).dump_json(
            outcome, by_alias=True, exclude_unset=True
        ).decode()


def apply_settings(run: Run) -> None:
    litellm.drop_params = run["drop_params"]
    litellm.api_key = run["settings"].get("api_key")
    litellm.api_version = run["settings"].get("api_version")


# the wrapper keywords are absent when the case left them out, and only extra_body tells absent from null
def request_values(run: Run, api_base: str) -> dict[str, Json]:
    values: dict[str, Json] = {"api_base": api_base}
    params = run.get("params")
    if params is not None:
        values.update(params)
    values["custom_llm_provider"] = run["provider"]
    values["thread_id"] = run["thread_id"]
    return values


def append_value(run: Run, api_base: str) -> dict[str, Json]:
    apply_settings(run)
    values = request_values(run, api_base)
    values["role"] = run["role"]
    values["content"] = run["content"]
    values["attachments"] = run["attachments"]
    values["metadata"] = run["metadata"]
    if run["mode"] == "async":
        message = asyncio.run(litellm.a_add_message(**values))
    else:
        message = litellm.add_message(**values)
    return JSON_DICT.validate_json(message.model_dump_json())


def list_value(run: Run, api_base: str) -> dict[str, Json]:
    apply_settings(run)
    values = request_values(run, api_base)
    if run["mode"] == "async":
        page = asyncio.run(litellm.aget_messages(**values))
    else:
        page = litellm.get_messages(**values)
    return JSON_DICT.validate_json(page.model_dump_json())


def failure_of(error: openai.APIError) -> Failure:
    failure = project_failure(error).failure
    if failure is None:
        raise ValueError("the SDK error projection carries no failure")
    return failure


def run_outcome(run: Run, api_base: str) -> RunOutcome:
    try:
        if run["operation"] == "list":
            return {"value": list_value(run, api_base)}
        return {"value": append_value(run, api_base)}
    except openai.APIError as error:
        return {"failure": failure_of(error)}
