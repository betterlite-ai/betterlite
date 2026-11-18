from typing import Literal, TypeAlias, TypedDict

import litellm
from litellm.proxy.pass_through_endpoints.passthrough_endpoint_router import (
    PassthroughEndpointRouter,
)
from litellm.router import Router
from litellm.types.passthrough_endpoints.vertex_ai import VertexPassThroughCredentials
from litellm.types.utils import CredentialItem
from pydantic import JsonValue, TypeAdapter

CREDENTIALS: TypeAdapter[list[CredentialItem]] = TypeAdapter(list[CredentialItem])


class VertexFields(TypedDict):
    vertex_project: str | None
    vertex_location: str | None
    vertex_credentials: str | dict[str, JsonValue] | None


VERTEX: TypeAdapter[VertexFields] = TypeAdapter(VertexFields)


class CommonInput(TypedDict):
    deployments: list[dict[str, JsonValue]]
    credentials: dict[str, dict[str, JsonValue]]
    environment: dict[str, str]


class CredentialsInput(CommonInput):
    function: Literal["credentials"]
    provider: str
    region: str | None


class VertexDeploymentsInput(CommonInput):
    function: Literal["vertex_deployments"]
    model: str | None


class StoredVertexCredential(TypedDict):
    project_id: str
    location: str
    credentials: str | dict[str, str] | None


class VertexKeyInput(CommonInput):
    function: Literal["vertex_key"]
    project_id: str | None
    location: str | None
    stored: list[StoredVertexCredential]


Input: TypeAlias = CredentialsInput | VertexDeploymentsInput | VertexKeyInput


class RouterGetter:
    def __init__(self, router: Router) -> None:
        self.router = router

    def __call__(self) -> Router:
        return self.router


def vertex_fields(credentials: VertexPassThroughCredentials | None) -> JsonValue:
    if credentials is None:
        return None
    fields = VERTEX.validate_json(credentials.model_dump_json())
    return {
        "vertex_project": fields["vertex_project"],
        "vertex_location": fields["vertex_location"],
        "vertex_credentials": fields["vertex_credentials"],
    }


def install_credentials(credentials: dict[str, dict[str, JsonValue]]) -> None:
    records: list[dict[str, JsonValue]] = [
        {
            "credential_name": name,
            "credential_info": {},
            "credential_values": values,
        }
        for name, values in credentials.items()
    ]
    litellm.credential_list = CREDENTIALS.validate_python(records)


def vertex_deployments_value(
    call: VertexDeploymentsInput, router: PassthroughEndpointRouter
) -> JsonValue:
    credentials = router.get_vertex_credentials_from_router_deployments(call["model"])
    return {"result": vertex_fields(credentials)}


def vertex_key_value(
    call: VertexKeyInput, router: PassthroughEndpointRouter
) -> JsonValue:
    for entry in call["stored"]:
        router.add_vertex_credentials(
            entry["project_id"], entry["location"], entry["credentials"]
        )
    credentials = router.get_vertex_credentials(call["project_id"], call["location"])
    return {"result": vertex_fields(credentials)}


def call_router(input_value: Input, router: PassthroughEndpointRouter) -> JsonValue:
    answer: JsonValue = None
    match input_value["function"]:
        case "credentials":
            answer = {
                "result": router.get_credentials(
                    input_value["provider"], input_value["region"]
                )
            }
        case "vertex_deployments":
            answer = vertex_deployments_value(input_value, router)
        case "vertex_key":
            answer = vertex_key_value(input_value, router)
    return answer


def run(input_value: Input) -> JsonValue:
    previous_credentials = litellm.credential_list
    try:
        install_credentials(input_value["credentials"])
        router = PassthroughEndpointRouter(
            llm_router_getter=RouterGetter(Router(model_list=input_value["deployments"]))
        )
        return call_router(input_value, router)
    finally:
        litellm.credential_list = previous_credentials
