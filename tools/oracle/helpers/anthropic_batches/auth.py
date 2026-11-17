from pydantic import JsonValue

from .api import AUTH_HEADERS, CONFIG
from .models import Auth, AuthScenario
from .wire import apply_environment, plain_headers


def auth_of(headers: dict[str, str], expected: str | None) -> JsonValue:
    for name in AUTH_HEADERS:
        if name in headers:
            return {"name": name, "matches": headers[name] == expected}
    return {"name": None, "matches": expected is None}


def auth_scenario(scenario: AuthScenario) -> JsonValue:
    apply_environment(scenario.environment)
    headers: dict[str, str] = dict(scenario.caller_headers)
    try:
        CONFIG.validate_environment(
            headers=headers,
            model="",
            messages=[],
            optional_params={},
            litellm_params={"api_base": scenario.params_api_base},
            api_key=scenario.api_key,
            api_base=scenario.api_base,
        )
    except ValueError as error:
        raised: dict[str, JsonValue] = {"kind": "ValueError", "message": str(error)}
        return {"headers": None, "auth": None, "raised": raised}
    return {
        "headers": plain_headers(headers),
        "auth": auth_of(headers, scenario.expected_auth),
        "raised": None,
    }


def auth_headers(payload: JsonValue) -> JsonValue:
    case = Auth.model_validate(payload)
    scenarios: list[JsonValue] = [auth_scenario(scenario) for scenario in case.scenarios]
    return {"scenarios": scenarios}
