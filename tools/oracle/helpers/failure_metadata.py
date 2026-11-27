import sys
from types import ModuleType
from typing import Final, Literal, TypeAlias, TypedDict

from litellm.proxy.hooks.proxy_track_cost_callback import (
    _get_request_tags_for_cost_tracking,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/proxy_track_cost_callback.py#L679
    _proxy_stamped_used_client_oauth_token,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/proxy_track_cost_callback.py#L90
    _should_track_cost_callback,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/proxy_track_cost_callback.py#L635
    _write_spend_metadata_to_kwargs,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/proxy_track_cost_callback.py#L599
)
from pydantic import JsonValue, TypeAdapter

JSON_VALUE: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class ProxyServerStub(ModuleType):
    general_settings: dict[str, JsonValue]


# ProxyUpdateSpend.disable_spend_updates imports the server's settings at call time.
PROXY_SERVER_STUB: Final[ProxyServerStub] = ProxyServerStub("litellm.proxy.proxy_server")
sys.modules["litellm.proxy.proxy_server"] = PROXY_SERVER_STUB


class StampInput(TypedDict):
    function: Literal["stamp"]
    request_data: dict[str, JsonValue]
    request_route: str | None


class PatchInput(TypedDict):
    function: Literal["patch"]
    request_data: dict[str, JsonValue]
    metadata: dict[str, JsonValue]


class TagsInput(TypedDict):
    function: Literal["tags"]
    standard_logging_object: dict[str, JsonValue] | None
    metadata: dict[str, JsonValue]


class EligibilityInput(TypedDict):
    function: Literal["eligibility"]
    disable_spend_updates: bool
    user_api_key: str | None
    user_id: str | None
    team_id: str | None
    end_user_id: str | None
    agent_id: str | None
    call_type: str | None


Input: TypeAlias = StampInput | PatchInput | TagsInput | EligibilityInput


def stamp(input_value: StampInput) -> JsonValue:
    return _proxy_stamped_used_client_oauth_token(
        input_value["request_data"], input_value["request_route"]
    )


def patch_metadata(input_value: PatchInput) -> JsonValue:
    kwargs = input_value["request_data"]
    _write_spend_metadata_to_kwargs(kwargs, input_value["metadata"])
    return JSON_VALUE.validate_python(kwargs)


def tags(input_value: TagsInput) -> JsonValue:
    standard = input_value["standard_logging_object"]
    return JSON_VALUE.validate_python(
        _get_request_tags_for_cost_tracking(
            standard,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/hooks/proxy_track_cost_callback.py#L679-L682
            input_value["metadata"],
        )
    )


def eligibility(input_value: EligibilityInput) -> JsonValue:
    PROXY_SERVER_STUB.general_settings = {
        "disable_spend_updates": input_value["disable_spend_updates"]
    }
    return _should_track_cost_callback(
        user_api_key=input_value["user_api_key"],
        user_id=input_value["user_id"],
        team_id=input_value["team_id"],
        end_user_id=input_value["end_user_id"],
        call_type=input_value["call_type"],
        agent_id=input_value["agent_id"],
    )


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "stamp":
        return stamp(input_value)
    if input_value["function"] == "patch":
        return patch_metadata(input_value)
    if input_value["function"] == "tags":
        return tags(input_value)
    return eligibility(input_value)
