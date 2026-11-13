from typing import Literal, Protocol, TypeAlias, TypedDict, cast

from litellm.proxy._types import LiteLLM_JWTAuth
from litellm.proxy.auth.handle_jwt import JWTHandler
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Settings(TypedDict, total=False):
    team_id_jwt_field: str | None
    team_ids_jwt_field: str | None
    team_id_default: str | None
    team_alias_jwt_field: str | None
    org_id_jwt_field: str | None
    org_alias_jwt_field: str | None
    user_id_jwt_field: str | None
    user_email_jwt_field: str | None
    user_allowed_email_domain: str | None
    user_id_upsert: bool
    end_user_id_jwt_field: str | None
    agent_id_jwt_field: str | None
    object_id_jwt_field: str | None


class Scenario(TypedDict):
    settings: Settings
    trusted_issuers: list[str]
    claims: dict[str, Json]
    default_value: str | None
    valid_user_email: bool | None
    leeway: int


class Input(TypedDict):
    scenarios: list[Scenario]
    tokens: list[str]


class Projection(TypedDict):
    team_id: str | None
    team_ids: list[str]
    all_team_ids: list[str]
    team_alias: str | None
    end_user_id: str | None
    user_id: str | None
    user_email: str | None
    object_id: str | None
    agent_claim: str | None
    org_id: str | None
    org_alias: str | None
    is_required_team_id: bool
    is_enforced_email_domain: bool
    is_upsert_user_id: bool
    leeway: int


class Output(TypedDict):
    scenarios: list[Projection]
    unverified: list[Json]


class Issuer(TypedDict):
    issuer: str
    disable_audience_validation: Literal[True]


# the claim methods take a bare dict, so basedpyright cannot type their results through JWTHandler
class ClaimsHandler(Protocol):
    leeway: int

    def get_team_id(
        self, token: dict[str, Json], default_value: str | None
    ) -> str | None: ...

    def get_team_ids_from_jwt(self, token: dict[str, Json]) -> list[str]: ...

    def get_all_jwt_team_ids(self, token: dict[str, Json]) -> list[str]: ...

    def get_team_alias(
        self, token: dict[str, Json], default_value: str | None
    ) -> str | None: ...

    def get_end_user_id(
        self, token: dict[str, Json], default_value: str | None
    ) -> str | None: ...

    def get_user_id(
        self, token: dict[str, Json], default_value: str | None
    ) -> str | None: ...

    def get_user_email(
        self, token: dict[str, Json], default_value: str | None
    ) -> str | None: ...

    def get_object_id(
        self, token: dict[str, Json], default_value: str | None
    ) -> str | None: ...

    def get_agent_claim(self, token: dict[str, Json]) -> str | None: ...

    def get_org_id(
        self, token: dict[str, Json], default_value: str | None
    ) -> str | None: ...

    def get_org_alias(
        self, token: dict[str, Json], default_value: str | None
    ) -> str | None: ...

    def is_required_team_id(self) -> bool: ...

    def is_enforced_email_domain(self) -> bool: ...

    def is_upsert_user_id(self, *, valid_user_email: bool | None = None) -> bool: ...


def build_handler(scenario: Scenario) -> JWTHandler:
    handler = JWTHandler()
    issuers: list[Issuer] = [
        {"issuer": issuer, "disable_audience_validation": True}
        for issuer in scenario["trusted_issuers"]
    ]
    handler.update_environment(
        prisma_client=None,
        user_api_key_cache=UserApiKeyCache(),
        litellm_jwtauth=LiteLLM_JWTAuth(issuers=issuers, **scenario["settings"]),
        leeway=scenario["leeway"],
    )
    return handler


def project(handler: ClaimsHandler, scenario: Scenario) -> Projection:
    claims = scenario["claims"]
    default = scenario["default_value"]
    return {
        "team_id": handler.get_team_id(claims, default),
        "team_ids": handler.get_team_ids_from_jwt(claims),
        "all_team_ids": handler.get_all_jwt_team_ids(claims),
        "team_alias": handler.get_team_alias(claims, default),
        "end_user_id": handler.get_end_user_id(claims, default),
        "user_id": handler.get_user_id(claims, default),
        "user_email": handler.get_user_email(claims, default),
        "object_id": handler.get_object_id(claims, default),
        "agent_claim": handler.get_agent_claim(claims),
        "org_id": handler.get_org_id(claims, default),
        "org_alias": handler.get_org_alias(claims, default),
        "is_required_team_id": handler.is_required_team_id(),
        "is_enforced_email_domain": handler.is_enforced_email_domain(),
        "is_upsert_user_id": handler.is_upsert_user_id(
            valid_user_email=scenario["valid_user_email"]
        ),
        "leeway": handler.leeway,
    }


def run(input_value: Input) -> Output:
    handlers = [
        cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/handle_jwt.py#L238-L746
            ClaimsHandler, build_handler(scenario)
        )
        for scenario in input_value["scenarios"]
    ]
    projections = [
        project(handler, scenario)
        for handler, scenario in zip(handlers, input_value["scenarios"], strict=True)
    ]
    unverified = [
        cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/handle_jwt.py#L283-L302
            Json,
            JWTHandler.get_unverified_claims(token),  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/handle_jwt.py#L283-L302
        )
        for token in input_value["tokens"]
    ]
    return {"scenarios": projections, "unverified": unverified}
