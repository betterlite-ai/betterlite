from collections.abc import Callable
from typing import TypeAlias, cast

from litellm.models.user import LiteLLM_UserTable
from litellm.proxy._types import Member, MemberDeleteRequest
from litellm.proxy.management_helpers.bulk_user_deletion import (
    _addresses_member,  # pyright: ignore[reportPrivateUsage]
    _addresses_user,  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]
    _any_filter,  # pyright: ignore[reportPrivateUsage]
    _duplicate_member_indexes,  # pyright: ignore[reportPrivateUsage]
    _eq_filter,  # pyright: ignore[reportPrivateUsage]
    _in_filter,  # pyright: ignore[reportPrivateUsage]
    _same_email,  # pyright: ignore[reportPrivateUsage]
    _team_users_filter,  # pyright: ignore[reportPrivateUsage]
    _with_row_email,  # pyright: ignore[reportPrivateUsage]
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def request_from(input_value: dict[str, Json]) -> MemberDeleteRequest:
    return MemberDeleteRequest(
        user_id=cast("str | None", input_value["user_id"]),
        user_email=cast("str | None", input_value["user_email"]),
    )


def run_validate(input_value: dict[str, Json]) -> Json:
    return request_from(input_value).model_dump(mode="json")


def run_same_email(input_value: dict[str, Json]) -> Json:
    request = request_from(input_value)
    candidate_email = cast("str | None", input_value["candidate_email"])
    return _same_email(candidate_email, request)


def run_addresses_member(input_value: dict[str, Json]) -> Json:
    request = request_from(input_value)
    member = Member.model_validate(input_value["member"])
    return _addresses_member(member, request)


def run_addresses_user(input_value: dict[str, Json]) -> Json:
    request = request_from(input_value)
    user = LiteLLM_UserTable.model_validate(input_value["user"])
    return _addresses_user(user, request)


def run_with_row_email(input_value: dict[str, Json]) -> Json:
    request = request_from(input_value)
    email_of = cast("dict[str, str]", input_value["email_of"])
    filled = _with_row_email(request, email_of)
    return filled.model_dump(mode="json")


def run_duplicate_indexes(input_value: dict[str, Json]) -> Json:
    rows = cast("list[dict[str, Json]]", input_value["requests"])
    requests = [MemberDeleteRequest.model_validate(row) for row in rows]
    indexes = _duplicate_member_indexes(requests)
    return cast(Json, sorted(indexes))


def run_eq_filter(input_value: dict[str, Json]) -> Json:
    field = cast(str, input_value["field"])
    value = cast(str, input_value["value"])
    return cast(Json, dict(_eq_filter(field, value)))


def run_in_filter(input_value: dict[str, Json]) -> Json:
    field = cast(str, input_value["field"])
    values = cast("list[str]", input_value["values"])
    return cast(Json, dict(_in_filter(field, values)))


def run_team_users_filter(input_value: dict[str, Json]) -> Json:
    team_id = cast(str, input_value["team_id"])
    user_ids = cast("list[str]", input_value["user_ids"])
    return cast(Json, dict(_team_users_filter(team_id, user_ids)))


def run_any_filter(input_value: dict[str, Json]) -> Json:
    rows = cast("list[dict[str, Json]]", input_value["clauses"])
    clauses = [dict(row) for row in rows]
    return cast(Json, dict(_any_filter(*clauses)))


HANDLERS: dict[str, Callable[[dict[str, Json]], Json]] = {
    "validate": run_validate,
    "same_email": run_same_email,
    "addresses_member": run_addresses_member,
    "addresses_user": run_addresses_user,
    "with_row_email": run_with_row_email,
    "duplicate_indexes": run_duplicate_indexes,
    "eq_filter": run_eq_filter,
    "in_filter": run_in_filter,
    "team_users_filter": run_team_users_filter,
    "any_filter": run_any_filter,
}


def run(input_value: dict[str, Json]) -> Json:
    operation = cast(str, input_value["operation"])
    handler = HANDLERS.get(operation)
    if handler is None:
        raise ValueError(f"unknown member_removal_identity operation: {operation}")
    return handler(input_value)
