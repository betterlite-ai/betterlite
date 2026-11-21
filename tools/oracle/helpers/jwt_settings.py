from typing import Annotated, Final, Literal, TypeAlias, TypedDict

from litellm.proxy._types import LiteLLM_JWTAuth
from litellm.secret_managers.main import str_to_bool
from pydantic import Field, JsonValue, TypeAdapter, ValidationError

UNKNOWN_PREFIX: Final = "Invalid arguments provided: "
ALLOWED_INFIX: Final = ". Allowed arguments are: "
VALUE_ERROR_PREFIX: Final = "Value error, "


# two LiteLLM names pass 30 characters, so they get shorter keys and are read and written by alias
class Dump(TypedDict):
    admin_jwt_scope: str
    admin_allowed_routes: list[str]
    team_id_jwt_field: str | None
    team_id_upsert: bool
    team_ids_jwt_field: str | None
    upsert_sso_user_to_team: bool
    team_allowed_routes: list[str]
    team_id_default: str | None
    team_alias_jwt_field: str | None
    org_id_jwt_field: str | None
    org_alias_jwt_field: str | None
    user_id_jwt_field: str | None
    user_email_jwt_field: str | None
    user_allowed_email_domain: str | None
    user_roles_jwt_field: str | None
    user_allowed_roles: list[str] | None
    user_id_upsert: bool
    end_user_id_jwt_field: str | None
    agent_id_jwt_field: str | None
    mcp_client_id_jwt_field: str | None
    public_key_ttl: float
    public_key_stale_ttl: float
    public_allowed_routes: list[str]
    enforce_rbac: bool
    roles_jwt_field: str | None
    role_mappings: JsonValue
    object_id_jwt_field: str | None
    scope_mappings: JsonValue
    enforce_scope_based_access: bool
    team_model_access: Annotated[bool, Field(alias="enforce_team_based_model_access")]
    custom_validate: None
    jwt_litellm_role_map: JsonValue
    sync_user_role_and_teams: bool
    oidc_userinfo_endpoint: str | None
    oidc_userinfo_enabled: bool
    oidc_userinfo_cache_ttl: float
    virtual_key_claim_field: str | None
    virtual_key_mapping_cache_ttl: float
    unregistered_client: Annotated[str, Field(alias="unregistered_jwt_client_behavior")]
    routing_overrides: JsonValue
    team_claim_fallback: bool
    fallback_to_db_teams: bool
    issuers: JsonValue


class ErrorItem(TypedDict):
    type: str
    loc: list[int | str]
    msg: str


# pydantic keeps an unvalidated default such as 600 an int, so the typed dump reads TTLs as floats
DUMP: Final[TypeAdapter[Dump]] = TypeAdapter(Dump)
JSON_VALUE: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
# ValidationError.errors() carries the rejected input as Any, so its JSON form is read instead
ERRORS: Final[TypeAdapter[list[ErrorItem]]] = TypeAdapter(list[ErrorItem])


class Scenario(TypedDict):
    settings: dict[str, JsonValue]
    references: dict[str, str]
    environment: dict[str, str]
    issuers: list[str | None]


class Input(TypedDict):
    scenarios: list[Scenario]


class Probe(TypedDict):
    issuer: str | None
    issuer_config: JsonValue
    virtual_key_claim_field: str | None
    behavior: str


class Accepted(TypedDict):
    accepted: JsonValue
    virtual_key_mapping_configured: bool
    probes: list[Probe]


class FieldRejected(TypedDict):
    rejected: Literal["validation"]
    location: list[int | str]
    constraint: str | None


class KeysRejected(TypedDict):
    rejected: Literal["value_error"]
    invalid: list[str]
    allowed: list[str]


class MessageRejected(TypedDict):
    rejected: Literal["value_error"]
    message: str


Outcome: TypeAlias = Accepted | FieldRejected | KeysRejected | MessageRejected


class Output(TypedDict):
    outcomes: list[Outcome]


def run(input_value: Input) -> Output:
    outcomes = [outcome(scenario) for scenario in input_value["scenarios"]]
    return {"outcomes": outcomes}


# proxy_server.py resolves each top-level os.environ/ value with get_secret, which calls str_to_bool
def resolved(scenario: Scenario) -> dict[str, JsonValue]:
    settings = dict(scenario["settings"])
    for field, variable in scenario["references"].items():
        text = scenario["environment"][variable]
        flag = str_to_bool(text)
        settings[field] = text if flag is None else flag
    return settings


def outcome(scenario: Scenario) -> Outcome:
    try:
        auth = LiteLLM_JWTAuth(**resolved(scenario))
    except ValidationError as error:
        first = ERRORS.validate_json(error.json())[0]
        message = first["msg"].removeprefix(VALUE_ERROR_PREFIX)
        constraint = message if first["type"] == "value_error" else None
        return {"rejected": "validation", "location": first["loc"], "constraint": constraint}
    except ValueError as error:
        return value_error(str(error))
    probes = [probe(auth, issuer) for issuer in scenario["issuers"]]
    dump = DUMP.validate_json(auth.model_dump_json())
    return {
        "accepted": JSON_VALUE.validate_json(DUMP.dump_json(dump, by_alias=True)),
        "virtual_key_mapping_configured": auth.is_virtual_key_mapping_configured(),
        "probes": probes,
    }


# the unknown-key names come from a set, so only their sorted list is compared
def value_error(message: str) -> KeysRejected | MessageRejected:
    if not message.startswith(UNKNOWN_PREFIX):
        return {"rejected": "value_error", "message": message}
    listed = message.removeprefix(UNKNOWN_PREFIX).removesuffix(".")
    named, _, allowed = listed.partition(ALLOWED_INFIX)
    invalid = sorted(named.split(", "))
    return {"rejected": "value_error", "invalid": invalid, "allowed": allowed.split(", ")}


def probe(auth: LiteLLM_JWTAuth, issuer: str | None) -> Probe:
    config = auth.get_issuer_config(issuer)
    dumped = None if config is None else JSON_VALUE.validate_json(config.model_dump_json())
    return {
        "issuer": issuer,
        "issuer_config": dumped,
        "virtual_key_claim_field": auth.get_virtual_key_claim_field(issuer),
        "behavior": auth.get_unregistered_jwt_client_behavior(issuer).value,
    }
