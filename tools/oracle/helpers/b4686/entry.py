import asyncio
import sys
from types import ModuleType

import litellm
from litellm.proxy._experimental.mcp_server.tool_search import (
    handle_agent_search,
    handle_mcp_proxy_tool,
    handle_mcp_tool_call,
    handle_mcp_tool_search,
    handle_skill_search,
)
from mcp.types import (
    AudioContent,
    CallToolResult,
    ContentBlock,
    EmbeddedResource,
    ImageContent,
    ResourceLink,
    TextContent,
)
from pydantic import JsonValue, ValidationError

from b4686.doubles import (
    FakeAgentModule,
    FakeOperationsModule,
    FakePermissionModule,
    FakeProxyModule,
    FakeRbacModule,
    FakeSkillModule,
    FakeSkillsModule,
    Fakes,
)
from b4686.model import ARGUMENTS, JSON, Envelope
from b4686.readers import guardrail_of, query_of, top_k_of
from b4686.schema import Input


async def dispatch(input_value: Input, fakes: Fakes) -> CallToolResult:
    handler = input_value["handler"]
    if handler == "agent_search":
        return await handle_agent_search(
            query=query_of(input_value),
            top_k=top_k_of(input_value),
            user_api_key_dict=fakes.principal,
        )
    if handler == "skill_search":
        return await handle_skill_search(
            query=query_of(input_value),
            top_k=top_k_of(input_value),
            user_api_key_dict=fakes.principal,
        )
    if handler == "mcp_tool_search":
        return await handle_mcp_tool_search(
            query=query_of(input_value),
            top_k=top_k_of(input_value),
            user_api_key_dict=fakes.principal,
            client_ip=input_value.get("client_ip"),
            mcp_servers=input_value.get("mcp_servers"),
            mcp_auth_header=input_value.get("mcp_auth_header"),
            mcp_server_auth_headers=fakes.server_headers,
            oauth2_headers=fakes.oauth2_headers,
            raw_headers=fakes.raw_headers,
        )
    if handler == "mcp_proxy_tool":
        name = input_value.get("name")
        if name is None:
            name = ""
        arguments = input_value.get("arguments")
        if arguments is None:
            arguments = {}
        return await handle_mcp_proxy_tool(
            name=name,
            arguments=ARGUMENTS.validate_python(arguments),
            user_api_key_dict=fakes.principal,
            client_ip=input_value.get("client_ip"),
            mcp_servers=input_value.get("mcp_servers"),
            mcp_auth_header=input_value.get("mcp_auth_header"),
            mcp_server_auth_headers=fakes.server_headers,
            oauth2_headers=fakes.oauth2_headers,
            raw_headers=fakes.raw_headers,
        )
    if handler == "mcp_tool_call":
        tool_name = input_value.get("tool_name")
        if tool_name is None:
            tool_name = ""
        arguments = input_value.get("call_arguments")
        if arguments is None:
            arguments = {}
        return await handle_mcp_tool_call(
            tool_name=tool_name,
            arguments=arguments,
            user_api_key_dict=fakes.principal,
            client_ip=input_value.get("client_ip"),
            mcp_servers=input_value.get("mcp_servers"),
            mcp_auth_header=input_value.get("mcp_auth_header"),
            mcp_server_auth_headers=fakes.server_headers,
            oauth2_headers=fakes.oauth2_headers,
            raw_headers=fakes.raw_headers,
            requested_server_id=input_value.get("requested_server_id"),
            guardrail_context=guardrail_of(input_value),
        )
    raise ValueError(f"unknown handler: {handler}")


def text_block_json(text: str) -> dict[str, JsonValue]:
    try:
        parsed = JSON.validate_json(text)
    except ValidationError:
        return {"type": "text", "text": text}
    return {"type": "text", "json": parsed}


def content_json(block: ContentBlock) -> JsonValue:
    match block:
        case TextContent(text=text):
            return text_block_json(text)
        case ImageContent() | AudioContent() | ResourceLink() | EmbeddedResource():
            raise ValueError("the search handlers return text content blocks only")


def envelope(result: CallToolResult, fakes: Fakes) -> Envelope:
    return {
        "content": [content_json(block) for block in result.content],
        "isError": result.is_error,
        "observed": fakes.observed,
    }


def run(input_value: Input) -> Envelope:
    fakes = Fakes(input_value)
    modules: list[ModuleType] = [
        FakeProxyModule(fakes),
        FakeOperationsModule(fakes),
        FakeAgentModule(fakes),
        FakePermissionModule(fakes),
        FakeRbacModule(fakes),
        FakeSkillsModule(fakes),
        FakeSkillModule(fakes),
    ]
    for module in modules:
        sys.modules[module.__name__] = module
    litellm.agent_search_embedding_model = input_value.get("agent_embedding_model")
    litellm.skill_search_embedding_model = input_value.get("skill_embedding_model")
    litellm.mcp_tool_search = input_value.get("settings")
    result = asyncio.run(dispatch(input_value, fakes))
    return envelope(result, fakes)
