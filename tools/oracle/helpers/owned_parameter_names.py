from typing import Literal, TypedDict

from litellm.types import litellm_params as params
from litellm.types.utils import all_litellm_params, is_litellm_owned_kwarg

OWNERS: dict[str, type] = {
    "provider_connection": params.ProviderConnection,
    "bedrock_batch_connection": params.BedrockBatchConnection,
    "connection_settings": params.ConnectionSettings,
    "dispatch_options": params.DispatchOptions,
    "routing_options": params.RoutingOptions,
    "deployment_options": params.DeploymentOptions,
    "specialized_router_options": params.SpecializedRouterOptions,
    "caching_options": params.CachingOptions,
    "cost_options": params.CostOptions,
    "observability_options": params.ObservabilityOptions,
    "agentic_loop_options": params.AgenticLoopOptions,
    "guardrail_options": params.GuardrailOptions,
    "prompt_options": params.PromptOptions,
    "response_options": params.ResponseOptions,
    "control_options": params.ControlOptions,
    "mock_options": params.MockOptions,
    "litellm_options": params.LiteLLMOptions,
    "call_state": params.CallState,
    "agentic_loop_state": params.AgenticLoopState,
    "router_state": params.RouterState,
    "proxy_request_state": params.ProxyRequestState,
    "entrypoint_state": params.EntrypointState,
    "internal_state": params.InternalState,
}


class Input(TypedDict):
    function: Literal[
        "wire_name_projection", "owned_name_registry",
        "internal_prefix_membership", "subset_registries"
    ]
    name: str
    owner: str
    root: str | None
    names: list[str]


def run(input_value: Input) -> dict[str, object]:
    if input_value["function"] == "wire_name_projection":
        root = input_value["root"]
        return {
            "wire": dict(params.wire(input_value["name"])),
            "names": list(params.wire_names(OWNERS[input_value["owner"]])),
            "owned": None if root is None else list(params.owned_wire_names(OWNERS[root])),
        }
    if input_value["function"] == "owned_name_registry":
        return {
            "names": list(all_litellm_params),
            "owned": [is_litellm_owned_kwarg(name) for name in input_value["names"]],
        }
    if input_value["function"] == "internal_prefix_membership":
        return {"owned": [is_litellm_owned_kwarg(name) for name in input_value["names"]]}
    return {
        "agentic_loop": list(params.AGENTIC_LOOP_KWARG_NAMES),
        "bedrock_batch": list(params.BEDROCK_BATCH_KWARG_NAMES),
    }
