import os
from typing import TypeAlias, cast

from litellm.proxy.common_utils.html_forms.default_credentials_hint import (
    should_hide_default_credentials_hint,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def boolean_value(value: Json, name: str) -> bool:
    if not isinstance(value, bool):
        raise ValueError(f"{name} must be a boolean")
    return value


def between(html: str, opening: str, closing: str) -> str:
    start = html.index(opening) + len(opening)
    return html[start : html.index(closing, start)]


def element(html: str, opening: str, closing: str) -> str:
    start = html.index(opening)
    return html[start : html.index(closing, start) + len(closing)]


def tag_with(html: str, needle: str) -> str:
    at = html.index(needle)
    return html[html.rindex("<", 0, at) : html.index(">", at) + 1]


def normalized(block: str) -> str:
    return "\n".join(line.strip() for line in block.splitlines() if line.strip())


def lines(block: str) -> list[Json]:
    return [line.strip() for line in block.splitlines() if line.strip()]


def run(input_value: dict[str, Json]) -> Json:
    environment = object_value(input_value["environment"], "environment")
    general_settings = object_value(input_value["general_settings"], "general_settings")
    show_deprecation_banner = boolean_value(
        input_value["show_deprecation_banner"], "show_deprecation_banner"
    )
    for name, value in environment.items():
        os.environ[name] = string_value(value, f"environment.{name}")
    # ui_login reads the URLs when imported, so the environment comes first
    from litellm.proxy.common_utils.html_forms import ui_login  # noqa: PLC0415

    hide = should_hide_default_credentials_hint(general_settings)
    html = ui_login.build_ui_login_form(
        show_deprecation_banner=show_deprecation_banner,
        hide_default_credentials_hint=hide,
    )
    banner: dict[str, Json] | None = None
    if 'class="deprecation-banner"' in html:
        block = between(html, '<div class="deprecation-banner">', "</div>")
        banner = {"link": between(block, '<a href="', '">'), "inner": normalized(block)}
    hint_present = 'class="info-box"' in html
    documentation: Json = None
    default_credentials: Json = None
    sso: Json = None
    if hint_present:
        documentation = tag_with(html, "docs.litellm.ai")
        default_credentials = normalized(element(html, "<p>By default,", "</p>"))
        sso = normalized(element(html, "<p>Need to set UI credentials or SSO?", "</p>"))
    hint: dict[str, Json] = {
        "present": hint_present,
        "admin_code": hint_present and "<code>admin</code>" in html,
        "master_key_code": hint_present and "<code>MASTER_KEY</code>" in html,
        "documentation": documentation,
    }
    fields: dict[str, Json] = {
        "username_label": element(html, '<label for="username">', "</label>"),
        "username": tag_with(html, 'id="username"'),
        "password_label": element(html, '<label for="password">', "</label>"),
        "password": tag_with(html, 'id="password"'),
        "show_password": tag_with(html, 'id="show-password"'),
        "show_password_label": element(html, '<label for="show-password">', "</label>"),
        "submit": tag_with(html, 'value="Login"'),
    }
    texts: dict[str, Json] = {
        "title": normalized(between(html, "<title>", "</title>")),
        "logo": normalized(between(html, '<div class="logo">', "</div>")),
        "subtitle": normalized(between(html, '<p class="subtitle">', "</p>")),
        "default_credentials": default_credentials,
        "sso": sso,
    }
    result: dict[str, Json] = {
        "hide_default_credentials_hint": hide,
        "form_action": cast(str, ui_login.url_to_redirect_to),
        "new_login_url": ui_login.new_ui_login_url,
        "form_tag": tag_with(html, "action="),
        "banner": banner,
        "hint": hint,
        "fields": fields,
        "toggle_script": lines(between(html, "<script>", "</script>")),
        "styles": lines(between(html, "<style>", "</style>")),
        "texts": texts,
    }
    return result
