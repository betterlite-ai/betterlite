import asyncio
import math
from collections.abc import Awaitable, Callable, Iterator, Sequence
from functools import partial
from types import TracebackType
from typing import Literal, Self, TypeAlias, TypeVar, cast
from unittest.mock import patch

import anyio
from litellm.experimental_mcp_client import client as mcp_client
from litellm.types.mcp import MCPTransport
from mcp import ClientSession, MCPError
from mcp.types import (
    BlobResourceContents,
    GetPromptRequestParams,
    GetPromptResult,
    PaginatedRequestParams,
    Prompt,
    PromptMessage,
    ReadResourceResult,
    Resource,
    ResourceTemplate,
    ServerCapabilities,
    TextResourceContents,
)
from pydantic import AnyUrl, BaseModel, ConfigDict, Field

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Result = TypeVar("Result")
Item: TypeAlias = Prompt | Resource | ResourceTemplate
ModelItem = TypeVar("ModelItem", Prompt, Resource, ResourceTemplate)


class Strict(BaseModel):
    model_config = ConfigDict(extra="forbid")


class ErrorInput(Strict):
    code: int
    message: str


class PageInput(Strict):
    delay_ms: int = 0
    next_cursor: str | None = None
    items: list[object] = Field(default_factory=list)
    error: ErrorInput | None = None


class PromptInput(Strict):
    name: str
    arguments: dict[str, str] | None = None
    result: dict[str, object] | None = None
    error: ErrorInput | None = None


class ResourceInput(Strict):
    uri: str
    result: dict[str, object] | None = None
    error: ErrorInput | None = None


class Input(Strict):
    operation: Literal[
        "list_prompts", "list_resources", "list_resource_templates", "get_prompt", "read_resource"
    ]
    client_timeout_ms: int = 60_000
    listing_timeout_ms: int = 30_000
    raise_on_error: bool = False
    capabilities: dict[str, object] | None = None
    pages: list[PageInput] = Field(default_factory=list)
    prompt: PromptInput | None = None
    resource: ResourceInput | None = None


class Budget:
    def __init__(self) -> None:
        self.deadline_ms: float = math.inf
        self.elapsed_ms: float = 0


class BudgetExpiredError(Exception):
    pass


class FailAfter:
    def __init__(self, budget: Budget, seconds: float) -> None:
        self.budget = budget
        self.seconds = seconds

    def __enter__(self) -> Self:
        self.budget.deadline_ms = self.seconds * 1000.0
        return self

    def __exit__(
        self,
        kind: type[BaseException] | None,
        value: BaseException | None,
        trace: TracebackType | None,
    ) -> bool:
        if kind is BudgetExpiredError:
            raise TimeoutError("the MCP list deadline expired") from value
        return False


class ScriptedPage:
    def __init__(
        self,
        prompts: Sequence[Prompt] = (),
        resources: Sequence[Resource] = (),
        resource_templates: Sequence[ResourceTemplate] = (),
        next_cursor: str | None = None,
    ) -> None:
        self.prompts = prompts
        self.resources = resources
        self.resource_templates = resource_templates
        self.next_cursor = next_cursor


class ScriptedSession:
    def __init__(self, input_value: Input, budget: Budget) -> None:
        self.input = input_value
        self.budget = budget
        capabilities = input_value.capabilities
        self.server_capabilities: ServerCapabilities | None = (
            None if capabilities is None else ServerCapabilities.model_validate(capabilities)
        )
        self.pages: Iterator[PageInput] = iter(input_value.pages)
        self.cursors: list[str | None] = []
        self.returned: set[str] = set()
        self.collected: list[Item] = []
        self.stopped_by_deviation = False
        self.prompt_params: list[dict[str, Json]] = []
        self.resource_uris: list[str] = []

    async def _fetch(self, params: PaginatedRequestParams | None) -> PageInput:
        self.cursors.append(None if params is None else params.cursor)
        page = next(self.pages)
        self.budget.elapsed_ms += page.delay_ms
        if self.budget.elapsed_ms > self.budget.deadline_ms:
            self.stopped_by_deviation = True
            raise BudgetExpiredError
        if page.error is not None:
            raise MCPError(code=page.error.code, message=page.error.message)
        if page.next_cursor is not None:
            if page.next_cursor in self.returned:
                self.stopped_by_deviation = True
            self.returned.add(page.next_cursor)
        return page

    def _collect(self, page: PageInput, model: type[ModelItem]) -> list[ModelItem]:
        items = [model.model_validate(item) for item in page.items]
        self.collected.extend(cast(Sequence[Item], items))
        return items

    async def list_prompts(self, params: PaginatedRequestParams | None = None) -> ScriptedPage:
        page = await self._fetch(params)
        prompts = self._collect(page, Prompt)
        return ScriptedPage(prompts=prompts, next_cursor=page.next_cursor)

    async def list_resources(self, params: PaginatedRequestParams | None = None) -> ScriptedPage:
        page = await self._fetch(params)
        resources = self._collect(page, Resource)
        return ScriptedPage(resources=resources, next_cursor=page.next_cursor)

    async def list_resource_templates(
        self, params: PaginatedRequestParams | None = None
    ) -> ScriptedPage:
        page = await self._fetch(params)
        templates = self._collect(page, ResourceTemplate)
        return ScriptedPage(resource_templates=templates, next_cursor=page.next_cursor)

    async def get_prompt(self, name: str, arguments: dict[str, str] | None) -> GetPromptResult:
        prompt = self.input.prompt
        if prompt is None:
            raise ValueError("the case has no prompt input")
        self.prompt_params.append(prompt_params_json(name, arguments))
        if prompt.error is not None:
            raise MCPError(code=prompt.error.code, message=prompt.error.message)
        if prompt.result is None:
            raise ValueError("the prompt case has no result")
        return GetPromptResult.model_validate(prompt.result)

    async def read_resource(self, uri: str) -> ReadResourceResult:
        resource = self.input.resource
        if resource is None:
            raise ValueError("the case has no resource input")
        self.resource_uris.append(uri)
        if resource.error is not None:
            raise MCPError(code=resource.error.code, message=resource.error.message)
        if resource.result is None:
            raise ValueError("the resource case has no result")
        return ReadResourceResult.model_validate(resource.result)


class ScriptedClient(mcp_client.MCPClient):
    session: ScriptedSession

    def __init__(self, session: ScriptedSession, timeout: float) -> None:
        super().__init__(  # pyright: ignore[reportUnknownMemberType]
            server_url="https://mcp.example.com/mcp",
            transport_type=MCPTransport.http,
            timeout=timeout,
        )
        self.session = session

    async def run_with_session(
        self,
        operation: Callable[[ClientSession], Awaitable[Result]],
        *,
        quiet_on_error: bool = False,
    ) -> Result:
        _ = quiet_on_error
        return await operation(cast(ClientSession, self.session))


def item_json(item: Item) -> dict[str, Json]:
    dumped = item.model_dump(mode="json", by_alias=True, exclude_none=True)
    return cast(dict[str, Json], dumped)


def prompt_params_json(name: str, arguments: dict[str, str] | None) -> dict[str, Json]:
    value: dict[str, Json] = {"name": name}
    if arguments is not None:
        value["arguments"] = cast(Json, arguments)
    return value


def message_json(message: PromptMessage) -> dict[str, Json]:
    content = message.content.model_dump(mode="json", by_alias=True, exclude_none=True)
    return {"role": message.role, "content": cast(Json, content)}


def prompt_result_json(result: GetPromptResult) -> dict[str, Json]:
    value: dict[str, Json] = {"messages": [message_json(message) for message in result.messages]}
    if result.description is not None:
        value["description"] = result.description
    return value


def contents_json(contents: TextResourceContents | BlobResourceContents) -> dict[str, Json]:
    value: dict[str, Json] = {"uri": contents.uri}
    if contents.mime_type is not None:
        value["mimeType"] = contents.mime_type
    if isinstance(contents, TextResourceContents):
        value["text"] = contents.text
    if isinstance(contents, BlobResourceContents):
        value["blob"] = contents.blob
    return value


async def run_list(input_value: Input, client: ScriptedClient, session: ScriptedSession) -> Json:
    items: Sequence[Item]
    try:
        if input_value.operation == "list_prompts":
            items = await client.list_prompts(raise_on_error=input_value.raise_on_error)
        elif input_value.operation == "list_resources":
            items = await client.list_resources(raise_on_error=input_value.raise_on_error)
        else:
            items = await client.list_resource_templates(raise_on_error=input_value.raise_on_error)
    except (RuntimeError, TimeoutError):
        if not session.stopped_by_deviation:
            raise
        items = session.collected
    if session.stopped_by_deviation:
        items = session.collected
    return {"items": [item_json(item) for item in items], "cursors": cast(Json, session.cursors)}


async def dispatch(input_value: Input, client: ScriptedClient, session: ScriptedSession) -> Json:
    if input_value.operation.startswith("list"):
        return await run_list(input_value, client, session)
    if input_value.operation == "get_prompt":
        prompt = input_value.prompt
        if prompt is None:
            raise ValueError("the case has no prompt input")
        result = await client.get_prompt(
            GetPromptRequestParams(name=prompt.name, arguments=prompt.arguments)
        )
        return {"result": prompt_result_json(result), "params": cast(Json, session.prompt_params)}
    resource = input_value.resource
    if resource is None:
        raise ValueError("the case has no resource input")
    result = await client.read_resource(AnyUrl(resource.uri))
    return {
        "result": {"contents": [contents_json(contents) for contents in result.contents]},
        "uris": cast(Json, session.resource_uris),
    }


def run(input_value: dict[str, Json]) -> Json:
    parsed = Input.model_validate(input_value)
    budget = Budget()
    session = ScriptedSession(parsed, budget)
    client = ScriptedClient(session, parsed.client_timeout_ms / 1000.0)
    with (
        patch.object(anyio, "fail_after", partial(FailAfter, budget)),
        patch.object(mcp_client, "MCP_TOOL_LISTING_TIMEOUT", parsed.listing_timeout_ms / 1000.0),
    ):
        return asyncio.run(dispatch(parsed, client, session))
