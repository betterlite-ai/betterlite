import asyncio
from collections.abc import Coroutine
from typing import Final, Literal, NotRequired, Protocol, TypedDict, cast

import litellm
import openai
from pydantic import BaseModel, ConfigDict, TypeAdapter
from pydantic import JsonValue as Json

from openai_chat import Failure, project_failure

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


class MessageCall(Protocol):
    def __call__(self, **params: Json) -> BaseModel: ...


class AsyncMessageCall(Protocol):
    def __call__(self, **params: Json) -> Coroutine[None, None, BaseModel]: ...


ADD_MESSAGE: MessageCall = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/assistants/main.py#L892-L1013
    MessageCall,
    litellm.add_message,
)
A_ADD_MESSAGE: AsyncMessageCall = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/assistants/main.py#L841-L890
    AsyncMessageCall,
    litellm.a_add_message,
)
GET_MESSAGES: MessageCall = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/assistants/main.py#L1057-L1161
    MessageCall,
    litellm.get_messages,
)
A_GET_MESSAGES: AsyncMessageCall = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/assistants/main.py#L1014-L1056
    AsyncMessageCall,
    litellm.aget_messages,
)


def apply_settings(run: Run) -> None:
    litellm.drop_params = run["drop_params"]
    litellm.api_key = run["settings"].get("api_key")
    litellm.api_version = run["settings"].get("api_version")


# the wrapper keywords are absent when the case left them out, except extra_body
def request_values(run: Run, api_base: str) -> dict[str, Json]:
    values: dict[str, Json] = {"api_base": api_base}
    params = run.get("params")
    if params is not None:
        values.update(params)
    values["custom_llm_provider"] = run["provider"]
    values["thread_id"] = run["thread_id"]
    return values


def call_value(run: Run, api_base: str) -> dict[str, Json]:
    apply_settings(run)
    values = request_values(run, api_base)
    append = run["operation"] == "append"
    if append:
        values["role"] = run["role"]
        values["content"] = run["content"]
        values["attachments"] = run["attachments"]
        values["metadata"] = run["metadata"]
    if run["mode"] == "async":
        reply = asyncio.run(A_ADD_MESSAGE(**values) if append else A_GET_MESSAGES(**values))
    else:
        reply = ADD_MESSAGE(**values) if append else GET_MESSAGES(**values)
    return JSON_DICT.validate_json(reply.model_dump_json())


def run_outcome(run: Run, api_base: str) -> RunOutcome:
    try:
        return {"value": call_value(run, api_base)}
    except openai.APIError as error:
        failure = project_failure(error).failure
        if failure is None:
            raise ValueError("the SDK error projection carries no failure") from error
        return {"failure": failure}
