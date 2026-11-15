from collections.abc import Callable, Mapping
from typing import Literal, TypeAlias, TypedDict

from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.hooks import PROXY_HOOKS
from litellm.proxy.hooks.parallel_request_limiter_v3 import (
    RateLimitDescriptor,
    RateLimitedModel,
)
from pydantic import JsonValue, TypeAdapter


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


class Operations:
    def __init__(
        self,
        internal_usage_cache: LimiterCache,
        model_group_resolver: Callable[[str], str | None],
    ) -> None: ...

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

    def key_model_descriptor(
        self, auth: UserAPIKeyAuth, requested_model: str | None
    ) -> JsonValue:
        descriptors: list[RateLimitDescriptor] = []
        self._add_model_per_key_rate_limit_descriptor(auth, requested_model, descriptors)
        return {"descriptors": [descriptor_json(entry) for entry in descriptors]}

    def team_model_descriptor(
        self, auth: UserAPIKeyAuth, requested_model: str | None
    ) -> JsonValue:
        descriptors: list[RateLimitDescriptor] = []
        self._add_team_model_rate_limit_descriptor_from_metadata(
            auth, requested_model, descriptors
        )
        return {"descriptors": [descriptor_json(entry) for entry in descriptors]}

    def project_model_descriptor(
        self, auth: UserAPIKeyAuth, requested_model: str | None
    ) -> JsonValue:
        descriptors: list[RateLimitDescriptor] = []
        self._add_project_model_rate_limit_descriptor_from_metadata(
            auth, requested_model, descriptors
        )
        return {"descriptors": [descriptor_json(entry) for entry in descriptors]}

    def project_io_descriptors(
        self, auth: UserAPIKeyAuth, requested_model: str | None
    ) -> JsonValue:
        descriptors: list[RateLimitDescriptor] = []
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


LIMITER_CLASS: Callable[..., Operations] = TypeAdapter(
    Callable[..., Operations]
).validate_python(
    type("LimiterOperations", (PROXY_HOOKS["parallel_request_limiter"], Operations), {})
)


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


DescriptorSelection: TypeAlias = (
    OrganizationInput | KeyModelInput | TeamModelInput | ProjectModelInput | ProjectIoInput
)


def descriptor_answer(limiter: Operations, input_value: DescriptorSelection) -> JsonValue:
    auth = TypeAdapter(UserAPIKeyAuth).validate_python(input_value["auth"])
    requested = input_value["requested"]
    if input_value["function"] == "organization":
        payload = limiter.organization_descriptors(auth, requested)
    elif input_value["function"] == "key_model":
        payload = limiter.key_model_descriptor(auth, requested)
    elif input_value["function"] == "team_model":
        payload = limiter.team_model_descriptor(auth, requested)
    elif input_value["function"] == "project_model":
        payload = limiter.project_model_descriptor(auth, requested)
    else:
        payload = limiter.project_io_descriptors(auth, requested)
    return payload


def run(input_value: Input) -> JsonValue:
    limiter = LIMITER_CLASS(
        LimiterCache(), model_group_resolver=GroupMap(input_value["groups"])
    )
    if input_value["function"] == "rated_model":
        return limiter.rated_model(input_value["requested"])
    if input_value["function"] == "key_owns_tpm":
        return limiter.key_owns_tpm(
            input_value["request_metadata"], model_json(input_value["model"])
        )
    return descriptor_answer(limiter, input_value)
