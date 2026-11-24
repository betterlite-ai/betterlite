from typing import Literal, TypedDict

from litellm.proxy._types import LitellmUserRoles, UserAPIKeyAuth
from litellm.proxy.vector_store_endpoints import utils as vector_store_utils
from litellm.proxy.vector_store_endpoints.utils import (
    assert_proxy_admin_for_vector_store_index_management,
)
from pydantic import JsonValue
from starlette.exceptions import HTTPException

Operation = Literal["create", "delete", "update", "list"]


class GateInput(TypedDict):
    function: Literal["gate"]
    user_role: str | None
    operation: Operation


class LifecycleInput(TypedDict):
    function: Literal["lifecycle"]
    method: str
    path: str
    index_name: str


Input = GateInput | LifecycleInput


def gate(fields: GateInput) -> JsonValue:
    role = fields["user_role"]
    caller = UserAPIKeyAuth() if role is None else UserAPIKeyAuth(user_role=LitellmUserRoles(role))
    try:
        assert_proxy_admin_for_vector_store_index_management(caller, operation=fields["operation"])
    except HTTPException as error:
        return {"passed": False, "status": error.status_code}
    return {"passed": True, "status": None}


def lifecycle(fields: LifecycleInput) -> JsonValue:
    return vector_store_utils._is_vector_store_index_lifecycle_request(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/vector_store_endpoints/utils.py#L69
        request_method=fields["method"],
        request_path=fields["path"],
        index_name=fields["index_name"],
    )


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "gate":
        return gate(input_value)
    return lifecycle(input_value)
