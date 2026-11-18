from typing import ClassVar, Literal, TypeAlias, TypedDict

import litellm
from litellm.litellm_core_utils.credential_accessor import CredentialAccessor
from litellm.proxy.pass_through_endpoints import passthrough_endpoint_router as module
from litellm.proxy.pass_through_endpoints.passthrough_endpoint_router import (
    PassthroughEndpointRouter,
)
from litellm.router import Router
from litellm.types.passthrough_endpoints.vertex_ai import VertexPassThroughCredentials
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override


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


class RouterGetter:
    def __init__(self, router: Router) -> None:
        self.router = router

    def __call__(self) -> Router:
        return self.router


class CredentialValues(CredentialAccessor):
    values: ClassVar[dict[str, dict[str, JsonValue]]] = {}

    @staticmethod
    @override
    def get_credential_values(credential_name: str) -> dict[str, JsonValue]:
        values = CredentialValues.values
        if credential_name not in values:
            return {}
        return values[credential_name]


class SecretValues:
    def __init__(self, values: dict[str, str]) -> None:
        self.values = values

    def __call__(self, name: str) -> str | None:
        if name not in self.values:
            return None
        return self.values[name]


def vertex_fields(value: VertexPassThroughCredentials | None) -> VertexFields | None:
    if value is None:
        return None
    return VERTEX.validate_json(value.model_dump_json())


def run(input_value: Input) -> JsonValue:
    router_instance = Router(model_list=input_value["deployments"])
    router = PassthroughEndpointRouter(llm_router_getter=RouterGetter(router_instance))
    previous_accessor = module.CredentialAccessor
    previous_secret_lookup = module.get_secret_str
    CredentialValues.values = input_value["credentials"]
    module.CredentialAccessor = CredentialValues
    module.get_secret_str = SecretValues(input_value["environment"])
    try:
        if input_value["function"] == "credentials":
            result = router.get_credentials(
                input_value["provider"], input_value["region"]
            )
            return JSON.validate_python({"result": result})
        if input_value["function"] == "vertex_deployments":
            deployment_result = router.get_vertex_credentials_from_router_deployments(
                input_value["model"]
            )
            return JSON.validate_python({"result": vertex_fields(deployment_result)})
        for entry in input_value["stored"]:
            router.add_vertex_credentials(
                entry["project_id"], entry["location"], entry["credentials"]
            )
        key_result = router.get_vertex_credentials(
            input_value["project_id"], input_value["location"]
        )
        return JSON.validate_python({"result": vertex_fields(key_result)})
    finally:
        module.CredentialAccessor = previous_accessor
        module.get_secret_str = previous_secret_lookup
