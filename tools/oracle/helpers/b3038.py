from collections.abc import Callable, Mapping
from typing import Literal, Protocol, TypeAlias, TypedDict, runtime_checkable

from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.hooks import PROXY_HOOKS
from litellm.proxy.hooks.parallel_request_limiter_v3 import (
    RateLimitDescriptor,
    RateLimitedModel,
)
from pydantic import ConfigDict, JsonValue, TypeAdapter


class AuthFields(TypedDict, total=False):
    api_key: str
    org_id: str
    team_id: str
    project_id: str
    organization_rpm_limit: int
    organization_tpm_limit: int
    metadata: dict[str, JsonValue]
    model_max_budget: dict[str, JsonValue]
    team_metadata: dict[str, JsonValue]
    organization_metadata: dict[str, JsonValue]
    project_metadata: dict[str, JsonValue]


class ModelJson(TypedDict):
    requested: str
    group: str


class RatedModelInput(TypedDict):
    function: Literal["rated_model"]
    requested: str | None
    groups: dict[str, str]
    window_size: int


class DescriptorInput(TypedDict):
    auth: AuthFields
    requested: str | None
    groups: dict[str, str]
    window_size: int


class OrganizationInput(DescriptorInput):
    function: Literal["organization"]


class KeyModelInput(DescriptorInput):
    function: Literal["key_model"]


class TeamModelInput(DescriptorInput):
    function: Literal["team_model"]


class ProjectModelInput(DescriptorInput):
    function: Literal["project_model"]


class ProjectIoInput(DescriptorInput):
    function: Literal["project_io"]


class KeyOwnsTpmInput(TypedDict):
    function: Literal["key_owns_tpm"]
    groups: dict[str, str]
    request_metadata: dict[str, JsonValue]
    model: ModelJson | None


ModelFunction: TypeAlias = Literal[
    "key_model", "team_model", "project_model", "project_io"
]

Input: TypeAlias = (
    RatedModelInput
    | OrganizationInput
    | KeyModelInput
    | TeamModelInput
    | ProjectModelInput
    | ProjectIoInput
    | KeyOwnsTpmInput
)


class NoRedisCache:
    redis_cache: JsonValue = None


class LimiterCache:
    def __init__(self) -> None:
        self.dual_cache = NoRedisCache()


class GroupMap:
    def __init__(self, groups: dict[str, str]) -> None:
        self.groups = groups

    def __call__(self, model: str) -> str | None:
        return self.groups.get(model)


class Operations(Protocol):
    def _rate_limited_model(
        self, requested_model: str | None
    ) -> RateLimitedModel | None: ...

    def create_organization_rate_limit_descriptor(  # ast-grep-ignore: no-long-names
        self, user_api_key_dict: UserAPIKeyAuth, requested_model: str | None = None
    ) -> list[RateLimitDescriptor]: ...

    def _add_model_per_key_rate_limit_descriptor(  # ast-grep-ignore: no-long-names
        self,
        user_api_key_dict: UserAPIKeyAuth,
        requested_model: str | None,
        descriptors: list[RateLimitDescriptor],
    ) -> None: ...

    def _add_team_model_rate_limit_descriptor_from_metadata(  # ast-grep-ignore: no-long-names
        self,
        user_api_key_dict: UserAPIKeyAuth,
        requested_model: str | None,
        descriptors: list[RateLimitDescriptor],
    ) -> None: ...

    def _add_project_model_rate_limit_descriptor_from_metadata(  # ast-grep-ignore: no-long-names
        self,
        user_api_key_dict: UserAPIKeyAuth,
        requested_model: str | None,
        descriptors: list[RateLimitDescriptor],
    ) -> None: ...

    def add_project_io_token_rate_limit_descriptors_from_metadata(  # ast-grep-ignore: no-long-names
        self,
        user_api_key_dict: UserAPIKeyAuth,
        requested_model: str | None,
        descriptors: list[RateLimitDescriptor],
    ) -> None: ...

    def _key_owns_model_tpm_limit_from_request_metadata(  # ast-grep-ignore: no-long-names
        self, request_metadata: Mapping[str, JsonValue], model: RateLimitedModel | None
    ) -> bool: ...

    def rated_model(self, requested_model: str | None) -> JsonValue:
        model = self._rate_limited_model(requested_model)
        if model is None:
            return {"model": None}
        return {"model": {"requested": model.requested, "group": model.group}}

    def organization_descriptors(
        self, auth: UserAPIKeyAuth, requested_model: str | None
    ) -> JsonValue:
        descriptors = self.create_organization_rate_limit_descriptor(
            auth, requested_model
        )
        return {"descriptors": [descriptor_json(entry) for entry in descriptors]}

    def model_descriptors(
        self,
        function: ModelFunction,
        auth: UserAPIKeyAuth,
        requested_model: str | None,
    ) -> JsonValue:
        descriptors: list[RateLimitDescriptor] = []
        if function == "key_model":
            self._add_model_per_key_rate_limit_descriptor(
                auth, requested_model, descriptors
            )
        elif function == "team_model":
            self._add_team_model_rate_limit_descriptor_from_metadata(
                auth, requested_model, descriptors
            )
        elif function == "project_model":
            self._add_project_model_rate_limit_descriptor_from_metadata(
                auth, requested_model, descriptors
            )
        else:
            self.add_project_io_token_rate_limit_descriptors_from_metadata(
                auth, requested_model, descriptors
            )
        return {"descriptors": [descriptor_json(entry) for entry in descriptors]}

    def key_owns_tpm(
        self,
        request_metadata: Mapping[str, JsonValue],
        model: RateLimitedModel | None,
    ) -> JsonValue:
        owned = self._key_owns_model_tpm_limit_from_request_metadata(
            request_metadata, model
        )
        return {"owned": owned}


@runtime_checkable
class LimiterConstructor(Protocol):
    def __call__(
        self,
        internal_usage_cache: LimiterCache,
        *,
        model_group_resolver: Callable[[str], str | None],
    ) -> Operations: ...


def descriptor_json(descriptor: RateLimitDescriptor) -> JsonValue:
    rate_limit = descriptor["rate_limit"]
    if rate_limit is None:
        return {
            "key": descriptor["key"],
            "value": descriptor["value"],
            "rate_limit": None,
        }
    return {
        "key": descriptor["key"],
        "value": descriptor["value"],
        "rate_limit": {
            "requests_per_unit": rate_limit.get("requests_per_unit"),
            "tokens_per_unit": rate_limit.get("tokens_per_unit"),
            "max_parallel_requests": rate_limit.get("max_parallel_requests"),
            "window_size": rate_limit.get("window_size"),
        },
    }


def model_json(model: ModelJson | None) -> RateLimitedModel | None:
    if model is None:
        return None
    return RateLimitedModel(requested=model["requested"], group=model["group"])


def make_limiter(groups: dict[str, str]) -> Operations:
    constructor = TypeAdapter[LimiterConstructor](
        LimiterConstructor, config=ConfigDict(arbitrary_types_allowed=True)
    ).validate_python(PROXY_HOOKS["parallel_request_limiter"])
    return constructor(LimiterCache(), model_group_resolver=GroupMap(groups))


def run(input_value: Input) -> JsonValue:
    limiter = make_limiter(input_value["groups"])
    if input_value["function"] == "rated_model":
        return Operations.rated_model(limiter, input_value["requested"])
    if input_value["function"] == "key_owns_tpm":
        return Operations.key_owns_tpm(
            limiter, input_value["request_metadata"], model_json(input_value["model"])
        )
    auth = TypeAdapter(UserAPIKeyAuth).validate_python(input_value["auth"])
    if input_value["function"] == "organization":
        return Operations.organization_descriptors(
            limiter, auth, input_value["requested"]
        )
    return Operations.model_descriptors(
        limiter, input_value["function"], auth, input_value["requested"]
    )
