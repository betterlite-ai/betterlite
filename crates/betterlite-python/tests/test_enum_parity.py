import subprocess
import sys
from enum import Enum
from typing import Final, TypedDict

import pytest
from pydantic import JsonValue, TypeAdapter

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

DESCRIBE_SCRIPT = """
import json

import litellm.caching.caching as caching
import litellm.exceptions as exceptions
import litellm.types.caching as cache_types
import litellm.types.guardrails as guardrails
import litellm.types.integrations.slack_alerting as alerting
import litellm.types.llms.openai as openai
import litellm.types.mcp as mcp
import litellm.types.router as router

PERCENT = "%s"
ENUMS = {
    "MCPAuth": mcp.MCPAuth,
    "MCPTransport": mcp.MCPTransport,
    "GuardrailEventHooks": guardrails.GuardrailEventHooks,
    "GUARDRAIL_DEFINITION_LOCATION": guardrails.GUARDRAIL_DEFINITION_LOCATION,
    "PiiAction": guardrails.PiiAction,
    "Role": guardrails.Role,
    "LiteLLMCacheType": cache_types.LiteLLMCacheType,
    "CacheMode": caching.CacheMode,
    "RouterErrorTypes": router.RouterErrorTypes,
    "RequestType": router.RequestType,
    "RateLimitErrorCategory": exceptions.RateLimitErrorCategory,
    "ResponsesAPIStreamEvents": openai.ResponsesAPIStreamEvents,
    "AlertType": alerting.AlertType,
}

described = {}
for name, enum in ENUMS.items():
    described[name] = [
        {
            "name": member.name,
            "value": member.value,
            "texts": [str(member), format(member), f"{member}", PERCENT % member],
        }
        for member in enum
    ]
print(json.dumps(described))
"""

OUR_ENUMS: Final[dict[str, type[Enum]]] = {
    "MCPAuth": our_mcp.MCPAuth,
    "MCPTransport": our_mcp.MCPTransport,
    "GuardrailEventHooks": our_guardrails_params.GuardrailEventHooks,
    "GUARDRAIL_DEFINITION_LOCATION": our_guardrails_params.GUARDRAIL_DEFINITION_LOCATION,
    "PiiAction": our_guardrails_pii.PiiAction,
    "Role": our_guardrails_core.Role,
    "LiteLLMCacheType": our_cache_types.LiteLLMCacheType,
    "CacheMode": our_caching.CacheMode,
    "RouterErrorTypes": our_router_errors.RouterErrorTypes,
    "RequestType": our_router_routing.RequestType,
    "RateLimitErrorCategory": our_exceptions.RateLimitErrorCategory,
    "ResponsesAPIStreamEvents": our_event_types.ResponsesAPIStreamEvents,
    "AlertType": our_alerting.AlertType,
}


class DescribedMember(TypedDict):
    name: str
    value: JsonValue
    texts: tuple[str, str, str, str]


def member_values(enum: type[Enum]) -> list[JsonValue]:
    values: list[JsonValue] = []
    for member in enum:
        value: JsonValue = member.value
        values.append(value)
    return values


def member_texts(member: Enum) -> tuple[str, str, str, str]:
    percent = PERCENT_FORMAT % member
    return (str(member), format(member), f"{member}", percent)


@pytest.fixture(scope="module")
def described() -> dict[str, list[DescribedMember]]:
    completed = subprocess.run(
        [sys.executable, "-c", DESCRIBE_SCRIPT],
        check=True,
        capture_output=True,
        text=True,
    )
    return TypeAdapter(dict[str, list[DescribedMember]]).validate_json(completed.stdout)


@pytest.mark.parametrize(("name", "ours"), sorted(OUR_ENUMS.items()))
def test_enum_members(
    name: str, ours: type[Enum], described: dict[str, list[DescribedMember]]
) -> None:
    theirs = described[name]
    assert [member.name for member in ours] == [member["name"] for member in theirs]
    assert member_values(ours) == [member["value"] for member in theirs]
    for our_member, their_member in zip(ours, theirs, strict=True):
        assert member_texts(our_member) == their_member["texts"]


def test_mcp_auth_values(described: dict[str, list[DescribedMember]]) -> None:
    theirs = described["MCPAuth"]
    assert member_values(our_mcp.MCPAuth) == [member["value"] for member in theirs]


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


GUARDRAIL_PROBE: Final = """
import betterlite._integrations.custom_guardrail as our_guardrail
import litellm.integrations.custom_guardrail as their_guardrail
from betterlite.types.guardrails.params import GuardrailEventHooks as OurHooks
from litellm.types.guardrails import GuardrailEventHooks as TheirHooks


class Ours(our_guardrail.CustomGuardrail):
    def summary(self, response, baseline):
        request_data = {"metadata": {}}
        self._process_response(
            response, request_data, event_type=OurHooks.pre_call, original_inputs=baseline
        )
        records = request_data["metadata"]["standard_logging_guardrail_information"]
        return records[0]["guardrail_response"]


class Theirs(their_guardrail.CustomGuardrail):
    def summary(self, response, baseline):
        return self._summarize_guardrail_response(response, baseline, TheirHooks.pre_call)


baseline = {"messages": [{"role": "user", "content": "hi"}]}
unchanged = {"messages": [{"role": "user", "content": "hi"}], "model": "gpt-5"}
changed = {"messages": [{"role": "user", "content": "bye"}]}
ours = Ours(guardrail_name="probe")
theirs = Theirs(guardrail_name="probe")
ours_said = (ours.summary(unchanged, baseline), ours.summary(changed, baseline))
theirs_said = (theirs.summary(unchanged, baseline), theirs.summary(changed, baseline))
assert ours_said == theirs_said == ("allow", "mask"), (ours_said, theirs_said)
print("OK")
"""


def test_guardrail_pre_call() -> None:
    completed = subprocess.run(
        [sys.executable, "-c", GUARDRAIL_PROBE], capture_output=True, text=True, check=False
    )
    assert completed.returncode == 0, completed.stderr
    assert completed.stdout.strip() == "OK"
