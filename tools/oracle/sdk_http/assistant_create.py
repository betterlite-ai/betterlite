import asyncio
from collections.abc import Coroutine
from typing import Final, Literal, Protocol, TypedDict, Unpack

import httpx
import litellm
import openai
from openai.types.beta.assistant import Assistant
from pydantic import BaseModel, ConfigDict, TypeAdapter
from pydantic import JsonValue as Json

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

    def given(self) -> dict[str, float]:
        kept: dict[str, float] = {}
        if self.connect is not None:
            kept["connect"] = self.connect
        if self.read is not None:
            kept["read"] = self.read
        if self.write is not None:
            kept["write"] = self.write
        if self.pool is not None:
            kept["pool"] = self.pool
        return kept


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
    timeout: float | str | Phases | None = None
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
    tools: list[dict[str, Json]] | None = None
    tool_resources: dict[str, Json] | None = None
    metadata: dict[str, str] | None = None
    temperature: int | float | None = None
    top_p: int | float | None = None
    response_format: dict[str, str] | None = None
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


class AssistantCreateValue(BaseModel):
    value: Outcome


class AssistantCreateResult(BaseModel):
    model_config = ConfigDict(extra="forbid")

    assistant_create: AssistantCreateValue


class CreateOptions(TypedDict):
    model: str
    name: str | None
    description: str | None
    instructions: str | None
    tools: list[dict[str, Json]] | None
    tool_resources: dict[str, Json] | None
    metadata: dict[str, str] | None
    temperature: int | float | None
    top_p: int | float | None
    response_format: dict[str, str] | None
    api_key: str | None
    api_base: str | None
    api_version: str | None
    organization: str | None
    max_retries: int | None
    timeout: float | str | httpx.Timeout | None
    request_timeout: int | float | None
    azure_ad_token: str | None
    tenant_id: str | None
    client_id: str | None
    client_secret: str | None
    azure_username: str | None
    azure_password: str | None
    azure_scope: str | None
    async_create_assistants: bool | int | str | None
    extra_body: dict[str, Json] | None
    client: None


class AssistantCreateRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    assistant_create: Call

    def serialize(self, api_base: str) -> str:
        return AssistantCreateResult(
            assistant_create=AssistantCreateValue(
                value=run(self.assistant_create, api_base)
            )
        ).model_dump_json(by_alias=True, exclude_unset=True)


ASSISTANT_FIELDS: Final = TypeAdapter(AssistantView)


def to_timeout(timeout: float | str | Phases | None) -> float | str | httpx.Timeout | None:
    if isinstance(timeout, Phases):
        return httpx.Timeout(**timeout.given())
    return timeout


def create_options(
    call: Call, api_base: str, timeout: float | str | httpx.Timeout | None
) -> CreateOptions:
    return CreateOptions(
        model=call.model,
        name=call.name,
        description=call.description,
        instructions=call.instructions,
        tools=call.tools,
        tool_resources=call.tool_resources,
        metadata=call.metadata,
        temperature=call.temperature,
        top_p=call.top_p,
        response_format=call.response_format,
        api_key=call.api_key,
        api_base=api_base if call.api_base == "mock" else None,
        api_version=call.api_version,
        organization=call.organization,
        max_retries=call.max_retries,
        timeout=timeout,
        request_timeout=call.request_timeout,
        azure_ad_token=call.azure_ad_token,
        tenant_id=call.tenant_id,
        client_id=call.client_id,
        client_secret=call.client_secret,
        azure_username=call.azure_username,
        azure_password=call.azure_password,
        azure_scope=call.azure_scope,
        async_create_assistants=call.async_create_assistants,
        extra_body=call.extra_body if "extra_body" in call.model_fields_set else {},
        client=None,
    )


def project_assistant(assistant: Assistant) -> Outcome:
    return Outcome(
        assistant=ASSISTANT_FIELDS.validate_json(assistant.model_dump_json())
    )


def project_failure(error: openai.APIError) -> Outcome:
    fields: dict[str, str | int | None] = {
        "cls": type(error).__name__,
        "status": status_code(error),
        "message": error.message,
    }
    for name in ("type", "param", "code"):
        fields.update(optional_text(error, name))
    return Outcome(failure=Failure.model_validate(fields))


# both entries are unannotated **kwargs functions, so the calls go through the options they read
class CreateFacade(Protocol):
    def create_assistants(
        self,
        custom_llm_provider: Literal["openai", "azure"],
        **kwargs: Unpack[CreateOptions],
    ) -> Assistant | Coroutine[None, None, Assistant]: ...

    async def acreate_assistants(
        self,
        custom_llm_provider: Literal["openai", "azure"],
        **kwargs: Unpack[CreateOptions],
    ) -> Assistant: ...


FACADE: CreateFacade = litellm


def run(call: Call, api_base: str) -> Outcome:
    if call.client is not None:
        raise ScriptedClientError(call.client)
    timeout = to_timeout(call.timeout)
    options = create_options(call, api_base, timeout)
    try:
        if call.entry == "sync":
            answer = FACADE.create_assistants(call.provider, **options)
            match answer:
                case Assistant():
                    return project_assistant(answer)
                case Coroutine():
                    raise UnexpectedAnswerError
        held = FACADE.acreate_assistants(call.provider, **options)
        return project_assistant(asyncio.run(held))
    except openai.APIError as error:
        return project_failure(error)
