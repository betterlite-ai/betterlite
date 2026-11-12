from typing import Literal, TypeAlias, TypedDict, cast

from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.spend_tracking.spend_counter_batch import (
    admission_counter_keys,
    post_call_counter_keys,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class AdmissionInput(TypedDict):
    function: Literal["admission_counter_keys"]
    token: str | None
    team_id: str | None
    user_id: str | None
    org_id: str | None
    project_id: str | None
    end_user_id: str | None
    token_end_user_id: str | None


class PostCallInput(TypedDict):
    function: Literal["post_call_counter_keys"]
    token: Json
    team_id: Json
    user_id: Json
    org_id: Json
    end_user_id: Json
    project_id: Json
    tags: list[Json]
    model_access_groups: list[Json]


Input: TypeAlias = AdmissionInput | PostCallInput


def run(input_value: Input) -> list[str]:
    if input_value["function"] == "admission_counter_keys":
        auth = UserAPIKeyAuth(
            token=input_value["token"],
            team_id=input_value["team_id"],
            user_id=input_value["user_id"],
            org_id=input_value["org_id"],
            project_id=input_value["project_id"],
            end_user_id=input_value["token_end_user_id"],
        )
        return sorted(admission_counter_keys(auth, input_value["end_user_id"]))
    return sorted(
        post_call_counter_keys(
            token=cast("str | None", input_value["token"]),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/spend_tracking/spend_counter_batch.py#L180-L203
            team_id=cast("str | None", input_value["team_id"]),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/spend_tracking/spend_counter_batch.py#L180-L203
            user_id=cast("str | None", input_value["user_id"]),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/spend_tracking/spend_counter_batch.py#L180-L203
            org_id=cast("str | None", input_value["org_id"]),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/spend_tracking/spend_counter_batch.py#L180-L203
            end_user_id=cast("str | None", input_value["end_user_id"]),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/spend_tracking/spend_counter_batch.py#L180-L203
            tags=input_value["tags"],
            model_access_groups=input_value["model_access_groups"],
            project_id=cast("str | None", input_value["project_id"]),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/spend_tracking/spend_counter_batch.py#L180-L203
        )
    )
