from collections.abc import Callable
from typing import TypedDict, cast

import litellm.proxy._experimental.mcp_server.byok_oauth_endpoints as endpoints


class Input(TypedDict):
    server_name: str
    server_initial: str
    client_id: str
    redirect_uri: str
    code_challenge: str
    code_challenge_method: str
    state: str
    server_id: str
    access_items: list[str]
    help_url: str


def _escape(value: str) -> str:
    return (
        value.replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
        .replace('"', "&quot;")
        .replace("'", "&#x27;")
    )


def run(input_value: Input) -> object:
    builder = cast(
        Callable[[str, str, str, str, str, str, str, str, list[str], str], str],
        getattr(endpoints, "_build_" + "authorize_html"),
    )
    html = builder(
        input_value["server_name"], input_value["server_initial"], input_value["client_id"],
        input_value["redirect_uri"],
        input_value["code_challenge"],
        input_value["code_challenge_method"],
        input_value["state"],
        input_value["server_id"],
        input_value["access_items"],
        input_value["help_url"],
    )
    oauth_fields = (
        ("client_id", input_value["client_id"]),
        ("redirect_uri", input_value["redirect_uri"]),
        ("code_challenge", input_value["code_challenge"]),
        ("code_challenge_method", input_value["code_challenge_method"]),
        ("state", input_value["state"]),
        ("server_id", input_value["server_id"]),
    )
    return {
        "steps": ['id="s1"' in html, 'id="s2"' in html],
        "api_key": 'name="api_key"' in html,
        "duration": 'value="until_revoked"' in html and "selDur('24h'" in html,
        "access_items": all(_escape(item) in html for item in input_value["access_items"]),
        "help_link": 'class="help-link"' in html,
        "oauth_values_escaped": all(
            f'name="{name}"' in html and f'value="{_escape(value)}"' in html
            for name, value in oauth_fields
        ),
    }
