from typing import Literal, TypeAlias, TypedDict

from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.spend_tracking.spend_counter_batch import (
    admission_counter_keys,
    post_call_counter_keys,
)
from pydantic import JsonValue


class AdmissionInput(TypedDict):
    function: Literal["admission_counter_keys"]
    token: str | None
    team_id: str | None
    user_id: str | None
    org_id: str | None
    project_id: str | None
    end_user_id: str | None


class PostCallInput(TypedDict):
    function: Literal["post_call_counter_keys"]
    token: str | None
    team_id: str | None
    user_id: str | None
    org_id: str | None
    end_user_id: str | None
    project_id: str | None
    tags: list[JsonValue]
    model_access_groups: list[JsonValue]


Input: TypeAlias = AdmissionInput | PostCallInput


def run(input_value: Input) -> list[str]:
    if input_value["function"] == "admission_counter_keys":
        auth = UserAPIKeyAuth(
            token=input_value["token"],
            team_id=input_value["team_id"],
            user_id=input_value["user_id"],
            org_id=input_value["org_id"],
            project_id=input_value["project_id"],
        )
        return sorted(admission_counter_keys(auth, input_value["end_user_id"]))
    return sorted(
        post_call_counter_keys(
            token=input_value["token"],
            team_id=input_value["team_id"],
            user_id=input_value["user_id"],
            org_id=input_value["org_id"],
            end_user_id=input_value["end_user_id"],
            tags=input_value["tags"],
            model_access_groups=input_value["model_access_groups"],
            project_id=input_value["project_id"],
        )
    )
