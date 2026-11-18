import json
from typing import Literal, TypeAlias, TypedDict

import litellm
from litellm.proxy.pass_through_endpoints.passthrough_endpoint_router import (
    PassthroughEndpointRouter,
)
from litellm.router import Router
from litellm.types.passthrough_endpoints.vertex_ai import VertexPassThroughCredentials
from litellm.types.utils import CredentialItem
from pydantic import JsonValue, TypeAdapter


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


class VertexFields(TypedDict):
    vertex_project: str | None
    vertex_location: str | None
    vertex_credentials: str | dict[str, JsonValue] | None


VERTEX: TypeAdapter[VertexFields] = TypeAdapter(VertexFields)
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
CREDENTIAL_ITEMS: TypeAdapter[list[CredentialItem]] = TypeAdapter(list[CredentialItem])


class RouterGetter:
    def __init__(self, router: Router) -> None:
        self.router = router

    def __call__(self) -> Router:
        return self.router


def vertex_fields(value: VertexPassThroughCredentials | None) -> VertexFields | None:
    if value is None:
        return None
    return VERTEX.validate_json(value.model_dump_json())


def run(input_value: Input) -> JsonValue:
    router_instance = Router(model_list=input_value["deployments"])
    router = PassthroughEndpointRouter(llm_router_getter=RouterGetter(router_instance))
    previous_credentials = litellm.credential_list
    litellm.credential_list = CREDENTIAL_ITEMS.validate_json(
        json.dumps(
            [
                {
                    "credential_name": name,
                    "credential_info": {},
                    "credential_values": values,
                }
                for name, values in input_value["credentials"].items()
            ]
        )
    )
    try:
        if input_value["function"] == "credentials":
            result = router.get_credentials(
                input_value["provider"], input_value["region"]
            )
            return JSON.validate_json('{"result":' + json.dumps(result) + "}")
        if input_value["function"] == "vertex_deployments":
            deployment_result = router.get_vertex_credentials_from_router_deployments(
                input_value["model"]
            )
            return JSON.validate_json(
                '{"result":' + json.dumps(vertex_fields(deployment_result)) + "}"
            )
        for entry in input_value["stored"]:
            router.add_vertex_credentials(
                entry["project_id"], entry["location"], entry["credentials"]
            )
        key_result = router.get_vertex_credentials(
            input_value["project_id"], input_value["location"]
        )
        return JSON.validate_json(
            '{"result":' + json.dumps(vertex_fields(key_result)) + "}"
        )
    finally:
        litellm.credential_list = previous_credentials
