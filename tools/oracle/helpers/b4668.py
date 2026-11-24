import asyncio
from typing import Literal, NotRequired, TypedDict

from fastapi import HTTPException
from litellm.models.object_permission import LiteLLM_ObjectPermissionTable
from litellm.proxy._experimental.mcp_server.auth.user_api_key_auth_mcp import (
    MCPRequestHandler,
    UnloadableEntitlementError,
)
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.auth import auth_checks
from litellm.proxy.utils import PrismaClient
from pydantic import JsonValue


class RowFields(TypedDict):
    object_permission_id: str
    mcp_toolsets: NotRequired[list[str] | None]


class LoadNamedCall(TypedDict):
    function: Literal["load_named"]
    principal: str
    object_permission_id: str
    requires_fresh_policy: bool
    read: Literal["missing", "unreadable", "loaded"]
    row: RowFields | None


class ToolsetCall(TypedDict):
    function: Literal["toolset"]
    row: RowFields | None
    requires_fresh_policy: bool
    server_id: NotRequired[str | None]


class UnionCall(TypedDict):
    function: Literal["union"]
    direct: list[str] | None
    via_toolsets: list[str] | None


Input = LoadNamedCall | ToolsetCall | UnionCall


class RowView(TypedDict):
    object_permission_id: str
    mcp_toolsets: list[str] | None


AnswerValue = RowView | dict[str, list[str]] | list[str] | None


class Outcome(TypedDict):
    value: AnswerValue
    raised: Literal["unloadable_entitlement", "fault"] | None
    caused: bool


# The loader only forwards this client to the read this case scripts.
class StubClient(PrismaClient):
    def __init__(self) -> None:
        pass


class ScriptedRead:
    def __init__(
        self, read: Literal["missing", "unreadable", "loaded"], row: RowFields | None
    ) -> None:
        self.read = read
        self.row = row

    async def __call__(
        self, *, check_db_only: bool = False, **_kwargs: JsonValue
    ) -> LiteLLM_ObjectPermissionTable | None:
        if self.read == "loaded":
            fields = self.row
            if fields is None:
                raise ValueError("a loaded read needs a row")
            return row_of(fields)
        if not check_db_only:
            return None
        if self.read == "missing":
            raise HTTPException(
                status_code=403, detail="Referenced object permission does not exist"
            )
        raise RuntimeError("the object permission read failed")


def run(input_value: Input) -> Outcome:
    try:
        value = dispatch(input_value)
    except UnloadableEntitlementError as raised:
        return unloadable_outcome(raised)
    return {"value": value, "raised": None, "caused": False}


def dispatch(input_value: Input) -> AnswerValue:
    match input_value["function"]:
        case "load_named":
            return load_named(input_value)
        case "toolset":
            return asyncio.run(toolset(input_value))
        case "union":
            return union(input_value)


def load_named(call: LoadNamedCall) -> RowView:
    # the before-validator drops the marker from constructor input, so a caller sets it after
    token = UserAPIKeyAuth()
    token.requires_fresh_policy = call["requires_fresh_policy"]
    # the loader imports get_object_permission when it runs, so the stub is in place before the call
    auth_checks.get_object_permission = ScriptedRead(call["read"], call["row"])  # pyright: ignore[reportAttributeAccessIssue, reportAssignmentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L3982
    loaded = asyncio.run(
        MCPRequestHandler._load_named_object_permission(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/auth/user_api_key_auth_mcp.py#L2796
            call["principal"], call["object_permission_id"], StubClient(), token
        )
    )
    return row_view(loaded)


async def toolset(call: ToolsetCall) -> AnswerValue:
    row = row_of(call["row"])
    server_id = call.get("server_id")
    if server_id is None:
        grants = await MCPRequestHandler._toolset_tool_permissions(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/auth/user_api_key_auth_mcp.py#L2153
            row, requires_fresh_policy=call["requires_fresh_policy"]
        )
        return {server: sorted(tools) for server, tools in sorted(grants.items())}
    tools = await MCPRequestHandler._toolset_tools_for_server(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/auth/user_api_key_auth_mcp.py#L2183
        row, server_id, requires_fresh_policy=call["requires_fresh_policy"]
    )
    return None if tools is None else sorted(tools)


def union(call: UnionCall) -> AnswerValue:
    grants = MCPRequestHandler._union_tool_grants(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_experimental/mcp_server/auth/user_api_key_auth_mcp.py#L2197
        call["direct"], call["via_toolsets"]
    )
    return None if grants is None else sorted(grants)


def unloadable_outcome(raised: UnloadableEntitlementError) -> Outcome:
    return {
        "value": None,
        "raised": "unloadable_entitlement",
        "caused": raised.__cause__ is not None,
    }


def row_of(fields: RowFields | None) -> LiteLLM_ObjectPermissionTable | None:
    if fields is None:
        return None
    toolsets = fields.get("mcp_toolsets")
    return LiteLLM_ObjectPermissionTable(
        object_permission_id=fields["object_permission_id"], mcp_toolsets=toolsets
    )


def row_view(row: LiteLLM_ObjectPermissionTable) -> RowView:
    return {
        "object_permission_id": row.object_permission_id,
        "mcp_toolsets": row.mcp_toolsets,
    }
