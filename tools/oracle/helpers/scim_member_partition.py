from typing import Literal, TypeAlias, TypedDict

from litellm.proxy.management_endpoints.scim.scim_v2 import _AmbiguousMember, _ClassifiedGroupMember, _PartitionedMembers, _ResolvedUserMember, _SkippedGroupMember, _UnknownMember, _admitted_member_ids, _json_object_fields, _member_value, _normalized_member_type, _partition_classified_members, _roster_entries_named_by, _team_metadata_has_scim_provenance  # noqa: E501  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/scim/scim_v2.py#L487-L851
from litellm.types.proxy.management_endpoints.scim_v2 import SCIMMember
from pydantic import JsonValue, TypeAdapter

_JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)

SkipReason: TypeAlias = Literal["nested_group", "non_user_type", "existing_team"]


class ResolvedEntry(TypedDict):
    kind: Literal["resolved"]
    user_id: str


class SkippedEntry(TypedDict):
    kind: Literal["skipped"]
    value: str
    reason: SkipReason


class UnknownEntry(TypedDict):
    kind: Literal["unknown"]
    value: str


class AmbiguousEntry(TypedDict):
    kind: Literal["ambiguous"]
    value: str


Entry: TypeAlias = ResolvedEntry | SkippedEntry | UnknownEntry | AmbiguousEntry


class MemberInput(TypedDict):
    value: str
    type: str | None


class RemovalInput(TypedDict):
    value: str
    roster: list[str]
    resolved: list[str]


class Input(TypedDict):
    members: list[MemberInput]
    metadata: list[JsonValue]
    entries: list[Entry]
    created_ids: list[str]
    removals: list[RemovalInput]


def member_answers(members: list[MemberInput]) -> list[JsonValue]:
    answers: list[JsonValue] = []
    for member in members:
        record = SCIMMember(value=member["value"], type=member["type"])
        answers.append(
            {"value": _member_value(record), "type": _normalized_member_type(record)}
        )
    return answers


def metadata_answers(metadata: list[JsonValue]) -> list[JsonValue]:
    return [
        {
            "is_object": _json_object_fields(raw) is not None,
            "provenance": _team_metadata_has_scim_provenance(raw),
        }
        for raw in metadata
    ]


def classified_entry(entry: Entry) -> _ClassifiedGroupMember:
    if entry["kind"] == "resolved":
        return _ResolvedUserMember(user_id=entry["user_id"])
    if entry["kind"] == "skipped":
        return _SkippedGroupMember(value=entry["value"], reason=entry["reason"])
    if entry["kind"] == "unknown":
        return _UnknownMember(value=entry["value"])
    return _AmbiguousMember(value=entry["value"])


def classified_entries(entries: list[Entry]) -> list[_ClassifiedGroupMember]:
    classified: list[_ClassifiedGroupMember] = []
    classified.extend(map(classified_entry, entries))
    return classified


def partition_answer(partition: _PartitionedMembers) -> JsonValue:
    return _JSON.validate_python(
        {
            "resolved_ids": list(partition.resolved_ids),
            "skipped": [
                {"value": member.value, "reason": member.reason}
                for member in partition.skipped
            ],
            "unknown_ids": list(partition.unknown_ids),
            "ambiguous_values": list(partition.ambiguous_values),
        }
    )


def roster_answers(removals: list[RemovalInput]) -> list[JsonValue]:
    answers: list[JsonValue] = []
    for removal in removals:
        roster = frozenset(removal["roster"])
        entries = _roster_entries_named_by(
            removal["value"], roster, tuple(removal["resolved"])
        )
        answers.append(
            {"value": removal["value"], "entries": _JSON.validate_python(list(entries))}
        )
    return answers


def run(input_value: Input) -> JsonValue:
    classified = classified_entries(input_value["entries"])
    return {
        "members": member_answers(input_value["members"]),
        "metadata": metadata_answers(input_value["metadata"]),
        "partition": partition_answer(_partition_classified_members(classified)),
        "admitted": _JSON.validate_python(
            list(_admitted_member_ids(classified, frozenset(input_value["created_ids"])))
        ),
        "roster": roster_answers(input_value["removals"]),
    }
