import os
from typing import Literal, TypeAlias, TypedDict

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Team(TypedDict):
    id: str
    label: str


class Verification(TypedDict):
    page: Literal["verification"]
    verify_url: str
    browser_complete_token: str
    prefill_user_code: str | None


class Consent(TypedDict):
    page: Literal["consent"]
    client_origin: str
    user_id: str
    teams: list[Team]
    flow_handle: str
    complete_url: str


class Completion(TypedDict):
    page: Literal["completion"]


Render = Verification | Consent | Completion


class Input(TypedDict):
    environment: dict[str, str]
    renders: list[Render]


def between(html: str, opening: str, closing: str) -> str:
    start = html.index(opening) + len(opening)
    return html[start : html.index(closing, start)]


def element(html: str, opening: str, closing: str) -> str:
    start = html.index(opening)
    end = html.index(closing, start) + len(closing)
    return html[start:end]


def tag_with(html: str, needle: str) -> str:
    at = html.index(needle)
    start = html.rindex("<", 0, at)
    return html[start : html.index(">", at) + 1]


def normalized(block: str) -> str:
    kept = [line.strip() for line in block.splitlines() if line.strip()]
    return "\n".join(kept)


def optional_between(html: str, opening: str, closing: str) -> str | None:
    if opening not in html:
        return None
    return normalized(between(html, opening, closing))


def optional_element(html: str, opening: str, closing: str) -> str | None:
    if opening not in html:
        return None
    return normalized(element(html, opening, closing))


def verification_facts(html: str) -> Json:
    facts: dict[str, Json] = {
        "page": "verification",
        "referrer": None,
        "script": None,
        "heading": normalized(between(html, "<h1>", "</h1>")),
        "instructions": normalized(between(html, "<p>", "</p>")),
        "form": normalized(element(html, "<form ", ">")),
        "token_input": normalized(tag_with(html, 'name="browser_complete_token"')),
        "code_label": normalized(element(html, '<label for="user_code">', "</label>")),
        "code_input": normalized(tag_with(html, 'id="user_code"')),
        "submit": normalized(element(html, '<button type="submit">', "</button>")),
    }
    return facts


def consent_facts(html: str) -> Json:
    lifetime = int(between(html, "expires within ", " hours"))
    team_hidden: Json = None
    if '<input type="hidden" name="team_id"' in html:
        team_hidden = normalized(tag_with(html, 'name="team_id"'))
    logout: Json = "lite logout" if "<code>lite logout</code>" in html else None
    denied = normalized(
        element(html, '<button type="submit" name="decision" value="deny"', "</button>")
    )
    approved = normalized(
        element(html, '<button type="submit" name="decision" value="approve"', "</button>")
    )
    facts: dict[str, Json] = {
        "page": "consent",
        "referrer": normalized(tag_with(html, 'name="referrer"')),
        "script": None,
        "form": normalized(element(html, "<form ", ">")),
        "flow_input": normalized(tag_with(html, 'name="flow"')),
        "origin": normalized(between(html, "<code>", "</code>")),
        "user": normalized(between(html, "<strong>", "</strong>")),
        "lifetime_hours": lifetime,
        "logout_command": logout,
        "team_hidden": team_hidden,
        "team_attribution": optional_between(html, "<p>Requests are attributed to team ", "</p>"),
        "team_select": optional_element(html, '<select id="team_id"', "</select>"),
        "deny_button": denied,
        "approve_button": approved,
    }
    return facts


def completion_facts(html: str) -> Json:
    script_block = between(html, "<script>", "</script>")
    script: list[Json] = [
        line.strip() for line in script_block.splitlines() if line.strip()
    ]
    facts: dict[str, Json] = {
        "page": "completion",
        "referrer": None,
        "script": script,
        "heading": normalized(between(html, "<h1>", "</h1>")),
        "subtitle": normalized(element(html, '<p class="subtitle">', "</p>")),
        "success_badge": normalized(element(html, '<div class="success-header">', "</div>")),
        "instructions_badge": normalized(
            element(html, '<div class="instructions-header">', "</div>")
        ),
        "return_instruction": normalized(element(html, "<p>Return to your terminal", "</p>")),
        "status": normalized(element(html, '<div class="status">', "</div>")),
    }
    return facts


def render_page(render: Render) -> Json:
    if render["page"] == "verification":
        from litellm.proxy.management_endpoints import ui_sso  # noqa: PLC0415  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/html_forms/native_client_consent.py#L5
        page = ui_sso._render_cli_sso_verification_page(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/ui_sso.py#L624
            render["verify_url"], render["browser_complete_token"], render["prefill_user_code"]
        )
        return verification_facts(page)
    if render["page"] == "consent":
        from litellm.proxy.common_utils.html_forms import native_client_consent  # noqa: PLC0415  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/html_forms/native_client_consent.py#L5
        teams = [(team["id"], team["label"]) for team in render["teams"]]
        page = native_client_consent.render_native_client_consent_page(
            client_origin=render["client_origin"],
            user_id=render["user_id"],
            teams=teams,
            flow_handle=render["flow_handle"],
            complete_url=render["complete_url"],
        )
        return consent_facts(page)
    from litellm.proxy.common_utils.html_forms import cli_sso_success  # noqa: PLC0415  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_utils/html_forms/native_client_consent.py#L5
    return completion_facts(cli_sso_success.render_cli_sso_success_page())


def run(input_value: Input) -> Json:
    # the lifetime is a constant read at import, so the environment goes in first
    for name, value in input_value["environment"].items():
        os.environ[name] = value
    return [render_page(render) for render in input_value["renders"]]
