from typing import Literal

from typing_extensions import ReadOnly, TypedDict


class ReturnedUITokenObject(TypedDict):
    user_id: str
    key: str
    user_email: str | None
    user_role: str
    login_method: Literal["sso", "username_password"]
    premium_user: bool
    auth_header_name: str
    # ast-grep-ignore: no-long-names
    disabled_non_admin_personal_key_creation: bool
    server_root_path: str
    password_reset_required: ReadOnly[bool]


class ParsedOpenIDResult(TypedDict, total=False):
    user_email: str | None
    user_id: str | None
    user_role: str | None
