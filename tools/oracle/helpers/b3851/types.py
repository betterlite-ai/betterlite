from typing import Literal, NotRequired, TypeAlias, TypedDict

from litellm.types.utils import StandardCallbackDynamicParams

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Fields: TypeAlias = dict[str, Json]
Environment: TypeAlias = dict[str, str]


class ExporterInput(TypedDict):
    kind: NotRequired[str]
    endpoint: NotRequired[str | None]
    headers: NotRequired[str | None]
    requires_headers: NotRequired[bool]


class BaseInput(TypedDict):
    exporters: list[ExporterInput]
    mapper_names: list[str]
    resource_attributes: dict[str, str]


class AgentopsPresetInput(TypedDict):
    function: Literal["agentops_preset"]
    environment: Environment
    base: BaseInput


class NewrelicPresetInput(TypedDict):
    function: Literal["newrelic_preset"]
    environment: Environment
    base: BaseInput


class SignozPresetInput(TypedDict):
    function: Literal["signoz_preset"]
    environment: Environment
    base: BaseInput


class LangfusePresetInput(TypedDict):
    function: Literal["langfuse_preset"]
    environment: Environment
    base: BaseInput
    allow_missing_credentials: bool


class NewrelicDynamicHeadersInput(TypedDict):
    function: Literal["newrelic_dynamic_headers"]
    params: StandardCallbackDynamicParams


class NewrelicDynamicEndpointInput(TypedDict):
    function: Literal["newrelic_dynamic_endpoint"]
    params: StandardCallbackDynamicParams


class SignozDynamicHeadersInput(TypedDict):
    function: Literal["signoz_dynamic_headers"]
    params: StandardCallbackDynamicParams
    allowed_hosts: list[str]


class SignozDynamicEndpointInput(TypedDict):
    function: Literal["signoz_dynamic_endpoint"]
    params: StandardCallbackDynamicParams
    allowed_hosts: list[str]


class LangfuseDynamicHeadersInput(TypedDict):
    function: Literal["langfuse_dynamic_headers"]
    params: StandardCallbackDynamicParams


class PresetForInput(TypedDict):
    function: Literal["preset_for"]
    callback_name: str


class DynamicOtlpHeadersInput(TypedDict):
    function: Literal["dynamic_otlp_headers"]
    callback_name: str | None
    params: StandardCallbackDynamicParams
    allowed_hosts: list[str]


class DynamicOtlpEndpointInput(TypedDict):
    function: Literal["dynamic_otlp_endpoint"]
    callback_name: str | None
    params: StandardCallbackDynamicParams
    allowed_hosts: list[str]


class ProjectRoutingHeadersInput(TypedDict):
    function: Literal["project_routing_headers"]
    callback_name: str | None
    auth_metadata: dict[str, str] | None


class BackendsInput(TypedDict):
    function: Literal["destination_capable_backends"]


class DestinationForInput(TypedDict):
    function: Literal["destination_for"]
    environment: Environment
    allowed_hosts: list[str]
    callback_name: str
    params: StandardCallbackDynamicParams
    service_name: str | None


Input: TypeAlias = (
    AgentopsPresetInput
    | NewrelicPresetInput
    | SignozPresetInput
    | LangfusePresetInput
    | NewrelicDynamicHeadersInput
    | NewrelicDynamicEndpointInput
    | SignozDynamicHeadersInput
    | SignozDynamicEndpointInput
    | LangfuseDynamicHeadersInput
    | PresetForInput
    | DynamicOtlpHeadersInput
    | DynamicOtlpEndpointInput
    | ProjectRoutingHeadersInput
    | BackendsInput
    | DestinationForInput
)
