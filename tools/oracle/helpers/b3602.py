from typing import TypedDict
from unittest.mock import patch

import litellm
from litellm.litellm_core_utils.credential_accessor import CredentialAccessor
from litellm.proxy.pass_through_endpoints import passthrough_endpoint_router as module
from litellm.proxy.pass_through_endpoints.passthrough_endpoint_router import (
    PassthroughEndpointRouter,
)
from litellm.router import Router
from litellm.types.router import DeploymentTypedDict
from pydantic import JsonValue, TypeAdapter


class Input(TypedDict, total=False):
    function: str
    provider: str
    region: str | None
    model: str | None
    project_id: str | None
    location: str | None
    config: dict[str, JsonValue] | None
    deployments: list[DeploymentTypedDict]
    credentials: dict[str, dict[str, JsonValue]]
    stored: list[dict[str, JsonValue]]
    environment: dict[str, str]


class VertexFields(TypedDict):
    vertex_project: str | None
    vertex_location: str | None
    vertex_credentials: str | dict[str, JsonValue] | None


VERTEX: TypeAdapter[VertexFields] = TypeAdapter(VertexFields)
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


def run(input_value: Input) -> JsonValue:
    credentials = input_value.get("credentials", {})
    environment = input_value.get("environment", {})
    deployments = input_value.get("deployments", [])
    router = PassthroughEndpointRouter(
        llm_router_getter=lambda: Router(model_list=deployments),
    )
    with (
        patch.object(
            CredentialAccessor,
            "get_credential_values",
            side_effect=lambda name: credentials.get(name),
        ),
        patch.object(
            module,
            "get_secret_str",
            side_effect=lambda name: environment.get(name),
        ),
    ):
        function = input_value.get("function", "")
        if function == "credentials":
            result = router.get_credentials(
                input_value.get("provider", ""), input_value.get("region")
            )
            return JSON.validate_python({"result": result})
        if function == "vertex_deployments":
            result = router.get_vertex_credentials_from_router_deployments(
                input_value.get("model")
            )
            fields = (
                None
                if result is None
                else VERTEX.validate_json(result.model_dump_json())
            )
            return JSON.validate_python({"result": fields})
        if function == "vertex_default":
            router.set_default_vertex_config(input_value.get("config"))
            result = router.get_vertex_credentials(None, None)
            fields = (
                None
                if result is None
                else VERTEX.validate_json(result.model_dump_json())
            )
            return JSON.validate_python({"result": fields})
        if function == "vertex_key":
            for entry in input_value.get("stored", []):
                router.add_vertex_credentials(
                    str(entry["project_id"]),
                    str(entry["location"]),
                    entry.get("credentials"),
                )
            result = router.get_vertex_credentials(
                input_value.get("project_id"), input_value.get("location")
            )
            fields = (
                None
                if result is None
                else VERTEX.validate_json(result.model_dump_json())
            )
            return JSON.validate_python({"result": fields})
        raise ValueError(f"unknown b3602 operation {function} ({litellm.__name__})")
