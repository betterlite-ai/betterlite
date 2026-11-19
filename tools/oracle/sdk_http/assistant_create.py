import asyncio
from typing import Final, Literal, TypedDict

import httpx
import litellm
import openai
from openai.types.beta.assistant import Assistant
from pydantic import BaseModel, ConfigDict, JsonValue as Json, TypeAdapter

from openai_chat import Failure, optional_text, status_code


class ScriptedClientError(ValueError):
    def __init__(self, client: str) -> None:
        super().__init__(f"the oracle cannot pass the {client} client object to LiteLLM")


class UnexpectedAnswerError(TypeError):
    def __init__(self) -> None:
        super().__init__("litellm.create_assistants returned a coroutine for a sync call")


class Phases(BaseModel):
    model_config = ConfigDict(extra="forbid")

    connect: float | None = None
    read: float | None = None
    write: float | None = None
    pool: float | None = None


class Call(BaseModel):
    model_config = ConfigDict(extra="allow")

    provider: Literal["openai", "azure"]
    model: str
    entry: Literal["sync", "async"] = "sync"
    api_base: Literal["mock", "absent"] = "mock"
    api_key: str | None = None
    api_version: str | None = None
    organization: str | None = None
    max_retries: int | None = None
    timeout: int | float | str | Phases | None = None
    request_timeout: int | float | None = None
    azure_ad_token: str | None = None
    tenant_id: str | None = None
    client_id: str | None = None
    client_secret: str | None = None
    azure_username: str | None = None
    azure_password: str | None = None
    azure_scope: str | None = None
    name: str | None = None
    description: str | None = None
    instructions: str | None = None
    tools: list[Json] | None = None
    tool_resources: Json | None = None
    metadata: dict[str, Json] | None = None
    temperature: int | float | None = None
    top_p: int | float | None = None
    response_format: Json | None = None
    client: str | None = None
    async_create_assistants: bool | int | str | None = None
    extra_body: dict[str, Json] | None = None


class AssistantView(TypedDict):
    id: str
    object: str
    model: str
    created_at: int
    name: str | None
    tools: list[Json]


class Outcome(BaseModel):
    model_config = ConfigDict(extra="forbid")

    assistant: AssistantView | None = None
    failure: Failure | None = None


class AssistantCreateRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    assistant_create: Call

    def serialize(self, api_base: str) -> str:
        return run(self.assistant_create, api_base).model_dump_json(
            by_alias=True, exclude_unset=True
        )


ASSISTANT_FIELDS: Final = TypeAdapter(AssistantView)


def to_timeout(
    timeout: int | float | str | Phases | None,
) -> float | str | httpx.Timeout | None:
    if not isinstance(timeout, Phases):
        return timeout
    phases: dict[str, float] = {}
    if timeout.connect is not None:
        phases["connect"] = timeout.connect
    if timeout.read is not None:
        phases["read"] = timeout.read
    if timeout.write is not None:
        phases["write"] = timeout.write
    if timeout.pool is not None:
        phases["pool"] = timeout.pool
    return httpx.Timeout(**phases)


def request_kwargs(call: Call, api_base: str) -> dict[str, Json]:
    kwargs: dict[str, Json] = {
        "model": call.model,
        "name": call.name,
        "description": call.description,
        "instructions": call.instructions,
        "tools": call.tools,
        "tool_resources": call.tool_resources,
        "metadata": call.metadata,
        "temperature": call.temperature,
        "top_p": call.top_p,
        "response_format": call.response_format,
        "api_key": call.api_key,
        "api_version": call.api_version,
        "organization": call.organization,
        "max_retries": call.max_retries,
        "request_timeout": call.request_timeout,
        "azure_ad_token": call.azure_ad_token,
        "tenant_id": call.tenant_id,
        "client_id": call.client_id,
        "client_secret": call.client_secret,
        "azure_username": call.azure_username,
        "azure_password": call.azure_password,
        "azure_scope": call.azure_scope,
        "async_create_assistants": call.async_create_assistants,
    }
    if call.api_base == "mock":
        kwargs["api_base"] = api_base
    # LiteLLM reads the environment token only when extra_body is present and null
    if "extra_body" in call.model_fields_set:
        kwargs["extra_body"] = call.extra_body
    return kwargs


def project_failure(error: openai.APIError) -> Outcome:
    fields: dict[str, str | int | None] = {
        "cls": type(error).__name__,
        "status": status_code(error),
        "message": error.message,
    }
    for name in ("type", "param", "code"):
        fields.update(optional_text(error, name))
    return Outcome(failure=Failure.model_validate(fields))


def project_success(assistant: Assistant) -> Outcome:
    return Outcome(assistant=ASSISTANT_FIELDS.validate_json(assistant.model_dump_json()))


def run(call: Call, api_base: str) -> Outcome:
    if call.client is not None:
        raise ScriptedClientError(call.client)
    kwargs: Final = request_kwargs(call, api_base)
    timeout: Final = to_timeout(call.timeout)
    try:
        if call.entry == "sync":
            answer = litellm.create_assistants(
                call.provider, **kwargs, client=None, timeout=timeout
            )
            if not isinstance(answer, Assistant):
                raise UnexpectedAnswerError
            return project_success(answer)
        held = litellm.acreate_assistants(
            call.provider, **kwargs, client=None, timeout=timeout
        )
        return project_success(asyncio.run(held))
    except openai.APIError as error:
        return project_failure(error)
