from enum import Enum
from typing import Final

import litellm.caching.caching as their_caching
import litellm.exceptions as their_exceptions
import litellm.types.caching as their_cache_types
import litellm.types.guardrails as their_guardrails
import litellm.types.integrations.slack_alerting as their_alerting
import litellm.types.llms.openai as their_openai
import litellm.types.mcp as their_mcp
import litellm.types.router as their_router
import pytest
from pydantic import JsonValue

import betterlite._router_types_errors as our_router_errors
import betterlite._router_types_routing as our_router_routing
import betterlite.caching as our_caching
import betterlite.exceptions as our_exceptions
import betterlite.types.caching as our_cache_types
import betterlite.types.guardrails.core as our_guardrails_core
import betterlite.types.guardrails.params as our_guardrails_params
import betterlite.types.guardrails.pii as our_guardrails_pii
import betterlite.types.integrations.slack_alerting as our_alerting
import betterlite.types.llms.openai.chat.event_types as our_event_types
import betterlite.types.mcp.enums as our_mcp

PERCENT_FORMAT: Final = "%s"

ENUM_PAIRS: Final[list[tuple[type[Enum], type[Enum]]]] = [
    (our_mcp.MCPAuth, their_mcp.MCPAuth),
    (our_mcp.MCPTransport, their_mcp.MCPTransport),
    (our_guardrails_params.GuardrailEventHooks, their_guardrails.GuardrailEventHooks),
    (
        our_guardrails_params.GUARDRAIL_DEFINITION_LOCATION,
        their_guardrails.GUARDRAIL_DEFINITION_LOCATION,
    ),
    (our_guardrails_pii.PiiAction, their_guardrails.PiiAction),
    (our_guardrails_core.Role, their_guardrails.Role),
    (our_cache_types.LiteLLMCacheType, their_cache_types.LiteLLMCacheType),
    (our_caching.CacheMode, their_caching.CacheMode),
    (our_router_errors.RouterErrorTypes, their_router.RouterErrorTypes),
    (our_router_routing.RequestType, their_router.RequestType),
    (our_exceptions.RateLimitErrorCategory, their_exceptions.RateLimitErrorCategory),
    (our_event_types.ResponsesAPIStreamEvents, their_openai.ResponsesAPIStreamEvents),
    (our_alerting.AlertType, their_alerting.AlertType),
]


def member_values(enum: type[Enum]) -> list[JsonValue]:
    values: list[JsonValue] = []
    for member in enum:
        value: JsonValue = member.value
        values.append(value)
    return values


def member_texts(member: Enum) -> tuple[str, str, str, str]:
    percent = PERCENT_FORMAT % member
    return (str(member), format(member), f"{member}", percent)


@pytest.mark.parametrize(("ours", "theirs"), ENUM_PAIRS)
def test_enum_members(ours: type[Enum], theirs: type[Enum]) -> None:
    assert [member.name for member in ours] == [member.name for member in theirs]
    assert member_values(ours) == member_values(theirs)
    for our_member, their_member in zip(ours, theirs, strict=True):
        assert member_texts(our_member) == member_texts(their_member)


def test_mcp_auth_values() -> None:
    values = member_values(our_mcp.MCPAuth)
    assert values == member_values(their_mcp.MCPAuth)


def test_rate_limit_error_text() -> None:
    error = our_exceptions.RateLimitError(
        message="rate limited",
        llm_provider="openai",
        model="gpt-5",
        category=our_exceptions.RateLimitErrorCategory.LITELLM_RATE_LIMIT,
        rate_limit_type=our_exceptions.RateLimitType.TOKENS,
    )
    assert error.category == "litellm_rate_limit"
    assert error.category.__class__ is str
    assert error.rate_limit_type == "tokens"
    assert error.rate_limit_type.__class__ is str


def test_cache_type_message() -> None:
    with pytest.raises(NotImplementedError) as caught:
        our_caching.Cache(type=our_cache_types.LiteLLMCacheType.S3)
    assert str(caught.value) == "s3 cache type is not built yet"
