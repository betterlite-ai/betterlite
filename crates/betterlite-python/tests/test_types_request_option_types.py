import subprocess
import sys
from typing import Final, TypedDict, TypeVar

import pytest
from pydantic import JsonValue, TypeAdapter

DESCRIBE_SCRIPT: Final = """
import ast
import importlib
import inspect
import json
import os
from enum import Enum
from typing import Literal, get_args, get_origin

from pydantic import BaseModel, TypeAdapter
from typing_extensions import is_typeddict

os.environ.setdefault("LITELLM_LOCAL_MODEL_COST_MAP", "True")

MODULES = {
    "files": ("betterlite.types.files", "litellm.types.files"),
    "rerank": ("betterlite.types.rerank", "litellm.types.rerank"),
    "embedding": ("betterlite.types.embedding", "litellm.types.embedding"),
    "fine_tuning": ("betterlite.types.fine_tuning", "litellm.types.fine_tuning"),
    "containers": ("betterlite.types.containers.main", "litellm.types.containers.main"),
    "google_genai": ("betterlite.types.google_genai.main", "litellm.types.google_genai.main"),
    "google_genai_adapters": (
        "betterlite.types.google_genai.adapters",
        "litellm.types.google_genai.adapters",
    ),
    "google_genai_package": ("betterlite.types.google_genai", "litellm.types.google_genai"),
    "images": ("betterlite.types.images.main", "litellm.types.images.main"),
    "realtime": ("betterlite.types.realtime", "litellm.types.realtime"),
}


def declared_names(module):
    discovered = []

    def walk(statements):
        for statement in statements:
            if isinstance(statement, (ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef)):
                discovered.append(statement.name)
            elif isinstance(statement, ast.Assign):
                for target in statement.targets:
                    if isinstance(target, ast.Name):
                        discovered.append(target.id)
            elif isinstance(statement, ast.AnnAssign) and isinstance(statement.target, ast.Name):
                discovered.append(statement.target.id)
            elif isinstance(statement, ast.If):
                walk(statement.body)
                walk(statement.orelse)

    walk(ast.parse(inspect.getsource(module)).body)
    return sorted(name for name in discovered if not name.startswith("_"))


def literal_arguments(annotation):
    if get_origin(annotation) is Literal:
        return [json.dumps(argument, sort_keys=True) for argument in get_args(annotation)]
    return [literal for part in get_args(annotation) for literal in literal_arguments(part)]


def literal_fields(annotations):
    return {
        name: literals
        for name, annotation in annotations.items()
        if (literals := literal_arguments(annotation))
    }


def empty_shape(kind):
    return {
        "kind": kind,
        "keys": [],
        "required": [],
        "defaults": {},
        "literals": {},
        "members": [],
        "readonly": [],
    }


def describe_typeddict(annotation):
    return {
        **empty_shape("typeddict"),
        "keys": sorted(annotation.__required_keys__ | annotation.__optional_keys__),
        "required": sorted(annotation.__required_keys__),
        "literals": literal_fields(annotation.__annotations__),
        "readonly": sorted(getattr(annotation, "__readonly_keys__", frozenset())),
    }


def describe_model(annotation):
    fields = annotation.model_fields
    return {
        **empty_shape("model"),
        "keys": sorted(fields),
        "required": sorted(name for name, field in fields.items() if field.is_required()),
        "defaults": {
            name: json.dumps(field.get_default(call_default_factory=True), sort_keys=True)
            for name, field in fields.items()
            if not field.is_required()
        },
        "literals": literal_fields({name: field.annotation for name, field in fields.items()}),
    }


def describe_enum(annotation):
    return {
        **empty_shape("enum"),
        "members": [[name, member.value] for name, member in annotation.__members__.items()],
    }


def describe(annotation):
    if is_typeddict(annotation):
        return describe_typeddict(annotation)
    if isinstance(annotation, type) and issubclass(annotation, Enum):
        return describe_enum(annotation)
    if isinstance(annotation, type) and issubclass(annotation, BaseModel):
        return describe_model(annotation)
    return empty_shape("other")


def outcome(call):
    try:
        call()
    except Exception as error:
        return {"ok": False, "error": type(error).__name__, "message": str(error)}
    return {"ok": True, "error": "", "message": ""}


def files_behavior(module):
    members = {member.name: member.value for member in module.FileType}
    extensions = {
        member.name: module.get_file_extension_for_file_type(member) for member in module.FileType
    }
    mime_types = {
        member.name: module.get_file_mime_type_for_file_type(member) for member in module.FileType
    }
    table = {
        member.name: [
            [
                extension,
                module.get_file_type_from_extension(extension).name,
                module.get_file_mime_type_from_extension(extension),
            ]
            for extension in module.FILE_EXTENSIONS[member]
        ]
        for member in module.FileType
    }
    by_mime = {
        member.name: module.get_file_extension_from_mime_type(module.FILE_MIME_TYPES[member])
        for member in module.FileType
    }
    groups = {
        name: sorted(member.name for member in getattr(module, name))
        for name in (
            "IMAGE_FILE_TYPES",
            "VIDEO_FILE_TYPES",
            "AUDIO_FILE_TYPES",
            "TEXT_FILE_TYPES",
            "GEMINI_1_5_ACCEPTED_FILE_TYPES",
        )
    }
    predicates = {
        member.name: [
            module.is_image_file_type(member),
            module.is_video_file_type(member),
            module.is_audio_file_type(member),
            module.is_text_file_type(member),
            module.is_gemini_1_5_accepted_file_type(member),
        ]
        for member in module.FileType
    }
    return {
        "members": members,
        "extensions": extensions,
        "mime_types": mime_types,
        "table": table,
        "by_mime": by_mime,
        "case": [
            module.get_file_type_from_extension("PNG").name,
            module.get_file_extension_from_mime_type("IMAGE/WEBP"),
            module.get_file_mime_type_from_extension("JPEG"),
        ],
        "unknown": [
            outcome(lambda: module.get_file_type_from_extension("nope")),
            outcome(lambda: module.get_file_extension_from_mime_type("nope/nope")),
        ],
        "groups": groups,
        "predicates": predicates,
    }


def rerank_behavior(module):
    request = module.RerankRequest(
        model="rerank", query="hi", documents=["a", {"text": "b"}], top_n=2, truncation_side="left"
    )
    ignored = module.RerankRequest(model="rerank", query="hi", documents=["a"], unknown="dropped")
    response = module.RerankResponse(
        id="r1",
        results=[{"index": 0, "relevance_score": 0.5, "document": {"text": "a"}}],
        meta={"api_version": {"version": 1}, "billed_units": {"search_units": 1}},
        unexpected="dropped",
    )
    response._hidden_params = {"model_id": "m"}
    truncation = module.HostedVLLMRerankTruncationParams(
        truncate_prompt_tokens=8, truncation_side="right", unexpected=1
    )
    return {
        "request": request.model_dump(),
        "ignored": ignored.model_dump(),
        "missing": outcome(lambda: module.RerankRequest(model="rerank", query="hi")),
        "side": outcome(
            lambda: module.RerankRequest(
                model="rerank", query="hi", documents=[], truncation_side="up"
            )
        ),
        "response": response.model_dump(),
        "hidden": response._hidden_params,
        "access": [
            response["id"],
            response.get("missing", "fallback"),
            "id" in response,
            "missing" in response,
        ],
        "difference": [outcome(lambda: response["missing"])],
        "truncation": truncation.model_dump(),
        "frozen": outcome(lambda: setattr(truncation, "truncate_prompt_tokens", 3)),
        "result": outcome(
            lambda: TypeAdapter(module.RerankResponseResult).validate_python(
                {"index": 1, "relevance_score": 0.25}
            )
        ),
        "result_missing": outcome(
            lambda: TypeAdapter(module.RerankResponseResult).validate_python({"index": 1})
        ),
        "optional": sorted(module.OptionalRerankParams.__optional_keys__),
    }


def embedding_behavior(module):
    return {
        "defaults": module.EmbeddingRequest(model="text-embedding-3-small").model_dump(),
        "extended": module.EmbeddingRequest(
            model="m", unknown="kept", custom_llm_provider={"provider": "x"}, timeout=12
        ).model_dump(),
        "missing": outcome(lambda: module.EmbeddingRequest()),
        "bad_input": outcome(lambda: module.EmbeddingRequest(model="m", input=[1])),
    }


def fine_tuning_behavior(module):
    return {
        "values": module.OpenAIFineTuningHyperparameters(
            batch_size="auto", learning_rate_multiplier=0.5, unknown=7
        ).model_dump(),
    }


def containers_behavior(module):
    expires = module.ExpiresAfter(anchor="last_active_at", minutes=30, unexpected=1)
    container = module.ContainerObject(
        id="c1",
        object="container",
        created_at=1,
        status="running",
        name="box",
        expires_after=expires,
    )
    container._hidden_params = {"model_id": "m"}
    extra = module.ContainerObject(
        id="c2", object="container", created_at=1, status="x", unexpected=1
    )
    file = module.ContainerFileObject(
        id="f1",
        object="container.file",
        container_id="c1",
        created_at=2,
        path="/a.txt",
        source="user",
        bytes=None,
    )
    underscored = module.ContainerFileObject(
        id="f2",
        object="container_file",
        container_id="c1",
        created_at=2,
        path="/b.txt",
        source="user",
    )
    listing = module.ContainerListResponse(object="list", data=[container], has_more=False)
    files = module.ContainerFileListResponse(
        object="list", data=[file], first_id="f1", has_more=False
    )
    deleted = module.DeleteContainerResult(id="c1", object="container.deleted", deleted=True)
    deleted_file = module.DeleteContainerFileResponse(
        id="f1", object="container_file.deleted", deleted=True
    )
    return {
        "expires": expires.model_dump(),
        "container": container.model_dump(),
        "container_json": container.json(),
        "container_json_exclude_none": container.json(exclude_none=True),
        "container_json_exclude_unset": container.json(exclude_unset=True),
        "container_get": container.get("missing", "fallback"),
        "container_item": container["id"],
        "container_hidden_item": container["_hidden_params"],
        "container_hidden_get": container.get("_hidden_params", "fallback"),
        "container_hidden_contains": "_hidden_params" in container,
        "container_contains": ["id" in container, "unknown" in container, "missing" in container],
        "container_missing_item": outcome(lambda: container["missing"]),
        "container_extra": extra.model_dump(),
        "file": file.model_dump(),
        "underscored": underscored.model_dump(),
        "listing": listing.model_dump(),
        "files": files.model_dump(),
        "deleted": deleted.model_dump(),
        "deleted_file": deleted_file.model_dump(),
        "bad_object": outcome(
            lambda: module.ContainerObject(id="c3", object="other", created_at=1, status="x")
        ),
        "bad_anchor": outcome(lambda: module.ExpiresAfter(anchor="created_at", minutes=1)),
        "hidden": container._hidden_params,
        "difference": [
            outcome(lambda: container.json(encoder=str)),
            outcome(lambda: container.json("x")),
        ],
    }


def google_behavior(module):
    response = module.GenerateContentResponse(prompt="hi", _hidden_params={"model_id": "m"})
    plain = module.GenerateContentResponse()
    request = module.GenerateContentRequestDict(
        generationConfig={"temperature": 0.5},
        tools=[{"name": "x"}],
        contents=[{"role": "user"}],
    )
    return {
        "common": {
            "response_hidden": response._hidden_params,
            "plain_dump": plain.model_dump(),
            "plain_hidden": plain._hidden_params,
            "request_dict": dict(request),
            "request_config": request.generationConfig,
            "request_tools": request.tools,
            "parameters_dict": dict(module.GenerateContentRequestParametersDict(contents=[])),
            "config_dict": dict(module.GenerateContentConfigDict(temperature=0.2)),
            "config_positional": outcome(
                lambda: module.GenerateContentConfigDict({"temperature": 0.2})
            ),
            "request_missing_tools": [module.GenerateContentRequestDict().tools],
        },
        "response_dump": response.model_dump(),
    }


def package_behavior(module):
    return {"all": list(module.__all__)}


def adapters_behavior(module):
    return {"optional": sorted(module.GenerateContentCompletionKwargs.__optional_keys__)}


def images_behavior(module):
    return {
        "optional": sorted(module.ImageEditOptionalRequestParams.__optional_keys__),
        "request": sorted(module.ImageEditRequestParams.__optional_keys__),
    }


def realtime_behavior(module):
    session = module.RealtimeSessionConfig(
        type="realtime",
        model="gpt-realtime",
        audio={"input": {"format": "pcm16"}},
        unknown="kept",
    )
    secret = module.RealtimeClientSecretRequest(
        expires_after={"anchor": "created_at", "seconds": 60},
        session=session,
        model="gpt-realtime",
        unknown="dropped",
    )
    secret_response = module.RealtimeClientSecretResponse(
        expires_at=10,
        value="ek_1",
        session={"client_secret": {"value": "ek_1"}},
        unknown="dropped",
    )
    transcription = module.RealtimeTranscriptionSessionRequest(
        input_audio_transcription={"model": "whisper-1"}, unknown="kept"
    )
    transcription_response = module.RealtimeTranscriptionSessionResponse(
        client_secret={"value": "ek_2"}, unknown="kept"
    )
    return {
        "session": session.model_dump(),
        "secret": secret.model_dump(),
        "secret_response": secret_response.model_dump(),
        "transcription": transcription.model_dump(),
        "transcription_response": transcription_response.model_dump(),
        "resolved": [
            module.RealtimeTranscriptionSessionRequest().resolved_model(),
            module.RealtimeTranscriptionSessionRequest(model="m1").resolved_model(),
            module.RealtimeTranscriptionSessionRequest(
                input_audio_transcription={"model": "m2"}
            ).resolved_model(),
            module.RealtimeTranscriptionSessionRequest(
                input_audio_transcription={}
            ).resolved_model(),
            module.RealtimeTranscriptionSessionRequest(
                model="m3", input_audio_transcription={"model": "m4"}
            ).resolved_model(),
        ],
        "missing_value": outcome(lambda: module.RealtimeClientSecretResponse(expires_at=1)),
        "delta_types": [
            outcome(lambda: TypeAdapter(module.ALL_DELTA_TYPES).validate_python("text")),
            outcome(lambda: TypeAdapter(module.ALL_DELTA_TYPES).validate_python("audio")),
            outcome(lambda: TypeAdapter(module.ALL_DELTA_TYPES).validate_python("video")),
        ],
        "expires": module.RealtimeExpiresAfter(unexpected=1).model_dump(),
        "expires_anchor": module.RealtimeExpiresAfter(anchor=None, unexpected=1).model_dump(),
        "error_event": TypeAdapter(module.RealtimeErrorEvent).validate_python(
            {"type": "error", "error": {"type": "invalid_request", "message": "bad"}}
        ),
    }


BEHAVIOR = {
    "files": files_behavior,
    "rerank": rerank_behavior,
    "embedding": embedding_behavior,
    "fine_tuning": fine_tuning_behavior,
    "containers": containers_behavior,
    "google_genai": google_behavior,
    "google_genai_adapters": adapters_behavior,
    "google_genai_package": package_behavior,
    "images": images_behavior,
    "realtime": realtime_behavior,
}


def module_report():
    described = {}
    for key, (ours_path, theirs_path) in MODULES.items():
        ours = importlib.import_module(ours_path)
        theirs = importlib.import_module(theirs_path)
        names = declared_names(theirs)
        described[key] = {
            "litellm": {
                name: None if not hasattr(theirs, name) else describe(getattr(theirs, name))
                for name in names
            },
            "betterlite": {
                name: None if not hasattr(ours, name) else describe(getattr(ours, name))
                for name in names
            },
        }
    return described


def behavior_report():
    described = {}
    for key, behavior in BEHAVIOR.items():
        ours = importlib.import_module(MODULES[key][0])
        theirs = importlib.import_module(MODULES[key][1])
        described[key] = {"litellm": behavior(theirs), "betterlite": behavior(ours)}
    return described


print(json.dumps({"modules": module_report(), "behavior": behavior_report()}))
"""


class Outcome(TypedDict):
    ok: bool
    error: str
    message: str


class Shape(TypedDict):
    kind: str
    keys: list[str]
    required: list[str]
    readonly: list[str]
    defaults: dict[str, str]
    literals: dict[str, list[str]]
    members: list[tuple[str, JsonValue]]


class ModuleShapes(TypedDict):
    litellm: dict[str, Shape | None]
    betterlite: dict[str, Shape | None]


class Report(TypedDict):
    modules: dict[str, ModuleShapes]
    behavior: dict[str, dict[str, JsonValue]]


Behavior = TypeVar("Behavior")


def described_shapes(report: Report, module: str) -> ModuleShapes:
    shapes = report["modules"][module]
    assert None not in shapes["litellm"].values()
    assert None not in shapes["betterlite"].values()
    return shapes


def module_shapes(report: Report, module: str) -> ModuleShapes:
    shapes = described_shapes(report, module)
    assert shapes["betterlite"] == shapes["litellm"]
    return shapes


def module_behavior(
    report: Report, module: str, adapter: TypeAdapter[Behavior],
) -> tuple[Behavior, Behavior]:
    behavior = report["behavior"][module]
    betterlite = adapter.validate_python(behavior["betterlite"])
    litellm = adapter.validate_python(behavior["litellm"])
    return betterlite, litellm


class FilesBehavior(TypedDict):
    members: dict[str, str]
    extensions: dict[str, str]
    mime_types: dict[str, str]
    table: dict[str, list[tuple[str, str, str]]]
    by_mime: dict[str, str]
    case: list[str]
    unknown: list[Outcome]
    groups: dict[str, list[str]]
    predicates: dict[str, list[bool]]


class RerankBehavior(TypedDict):
    request: dict[str, JsonValue]
    ignored: dict[str, JsonValue]
    missing: Outcome
    side: Outcome
    response: dict[str, JsonValue]
    hidden: dict[str, JsonValue]
    access: list[JsonValue]
    difference: list[JsonValue]
    truncation: dict[str, JsonValue]
    frozen: Outcome
    result: Outcome
    result_missing: Outcome
    optional: list[str]


class EmbeddingBehavior(TypedDict):
    defaults: dict[str, JsonValue]
    extended: dict[str, JsonValue]
    missing: Outcome
    bad_input: Outcome


class FineTuningBehavior(TypedDict):
    values: dict[str, JsonValue]


class ContainersBehavior(TypedDict):
    expires: dict[str, JsonValue]
    container: dict[str, JsonValue]
    container_json: dict[str, JsonValue]
    container_json_exclude_none: dict[str, JsonValue]
    container_json_exclude_unset: dict[str, JsonValue]
    container_get: str
    container_item: str
    container_hidden_item: dict[str, JsonValue]
    container_hidden_get: dict[str, JsonValue]
    container_hidden_contains: bool
    container_contains: list[bool]
    container_missing_item: Outcome
    container_extra: dict[str, JsonValue]
    file: dict[str, JsonValue]
    underscored: dict[str, JsonValue]
    listing: dict[str, JsonValue]
    files: dict[str, JsonValue]
    deleted: dict[str, JsonValue]
    deleted_file: dict[str, JsonValue]
    bad_object: Outcome
    bad_anchor: Outcome
    hidden: dict[str, JsonValue]
    difference: list[Outcome]


class GoogleCommon(TypedDict):
    response_hidden: dict[str, JsonValue]
    plain_dump: dict[str, JsonValue]
    plain_hidden: dict[str, JsonValue]
    request_dict: dict[str, JsonValue]
    request_config: dict[str, JsonValue]
    request_tools: list[JsonValue]
    parameters_dict: dict[str, JsonValue]
    config_dict: dict[str, JsonValue]
    config_positional: Outcome
    request_missing_tools: list[JsonValue]


class GoogleBehavior(TypedDict):
    common: GoogleCommon
    response_dump: dict[str, JsonValue]


class OptionalKeysBehavior(TypedDict):
    optional: list[str]


class PackageBehavior(TypedDict):
    all: list[str]


class ImagesBehavior(TypedDict):
    optional: list[str]
    request: list[str]


class RealtimeBehavior(TypedDict):
    session: dict[str, JsonValue]
    secret: dict[str, JsonValue]
    secret_response: dict[str, JsonValue]
    transcription: dict[str, JsonValue]
    transcription_response: dict[str, JsonValue]
    resolved: list[str | None]
    missing_value: Outcome
    delta_types: list[Outcome]
    expires: dict[str, JsonValue]
    expires_anchor: dict[str, JsonValue]
    error_event: dict[str, JsonValue]


@pytest.fixture(scope="module")
def report() -> Report:
    completed = subprocess.run(
        [sys.executable, "-c", DESCRIBE_SCRIPT], check=True, capture_output=True, text=True,
    )
    parsed: Report = TypeAdapter(Report).validate_json(completed.stdout)
    return parsed


FILE_TYPE_MEMBERS: Final = 42
RERANK_TOP_N: Final = 2
EMBEDDING_TIMEOUT: Final = 600


def test_files_names(report: Report) -> None:
    shapes = module_shapes(report, "files")
    file_type = shapes["betterlite"]["FileType"]
    assert file_type is not None
    assert file_type["kind"] == "enum"
    members = dict(file_type["members"])
    assert members["THREE_GPP"] == "3GPP"
    assert len(members) == FILE_TYPE_MEMBERS
    betterlite, litellm = module_behavior(report, "files", TypeAdapter(FilesBehavior))
    assert betterlite == litellm
    assert betterlite["mime_types"]["PDF"] == "application/pdf"
    assert betterlite["extensions"]["HTML"] == "html"
    assert betterlite["table"]["JPEG"] == [
        ("jpeg", "JPEG", "image/jpeg"),
        ("jpg", "JPEG", "image/jpeg"),
    ]
    assert betterlite["table"]["HTML"] == [
        ("html", "HTML", "text/html"),
        ("htm", "HTML", "text/html"),
    ]
    assert betterlite["by_mime"]["TXT"] == "txt"
    assert betterlite["case"] == ["PNG", "webp", "image/jpeg"]
    assert betterlite["groups"]["TEXT_FILE_TYPES"] == ["CSV", "HTML", "RTF", "TXT"]
    assert betterlite["predicates"]["PNG"] == [True, False, False, False, True]
    assert betterlite["predicates"]["MP4"] == [False, True, False, False, True]
    assert betterlite["unknown"][0]["error"] == "ValueError"
    assert betterlite["unknown"][0]["message"] == "Unknown file type for extension: nope"
    assert betterlite["unknown"][1]["message"] == "Unknown extension for mime type: nope/nope"


def test_rerank_names(report: Report) -> None:
    shapes = module_shapes(report, "rerank")
    request_shape = shapes["betterlite"]["RerankRequest"]
    truncation_shape = shapes["betterlite"]["HostedVLLMRerankTruncationParams"]
    optional_shape = shapes["betterlite"]["OptionalRerankParams"]
    assert request_shape is not None
    assert truncation_shape is not None
    assert optional_shape is not None
    assert request_shape["kind"] == "model"
    assert truncation_shape["kind"] == "model"
    assert optional_shape["kind"] == "typeddict"
    assert optional_shape["readonly"] == [
        "max_tokens_per_query", "truncate_prompt_tokens", "truncation_side",
    ]
    assert request_shape["required"] == ["documents", "model", "query"]
    assert request_shape["literals"] == {"truncation_side": ['"left"', '"right"']}
    betterlite, litellm = module_behavior(report, "rerank", TypeAdapter(RerankBehavior))
    shared_betterlite = {
        field: value for field, value in betterlite.items() if field != "difference"
    }
    shared_litellm = {field: value for field, value in litellm.items() if field != "difference"}
    assert shared_betterlite == shared_litellm
    assert betterlite["request"]["top_n"] == RERANK_TOP_N
    assert betterlite["request"]["truncation_side"] == "left"
    assert betterlite["request"]["documents"] == ["a", {"text": "b"}]
    assert "unknown" not in betterlite["ignored"]
    assert betterlite["missing"]["error"] == "ValidationError"
    assert "documents" in betterlite["missing"]["message"]
    assert "truncation_side" in betterlite["side"]["message"]
    assert "Instance is frozen" in betterlite["frozen"]["message"]
    assert betterlite["hidden"] == {"model_id": "m"}
    assert betterlite["access"] == ["r1", "fallback", True, False]
    missing_betterlite = TypeAdapter(Outcome).validate_python(betterlite["difference"][0])
    assert missing_betterlite["error"] == "AttributeError"
    missing_litellm = TypeAdapter(Outcome).validate_python(litellm["difference"][0])
    assert missing_litellm["error"] == "KeyError"
    assert "_hidden_params" not in betterlite["response"]
    assert "unexpected" not in betterlite["response"]
    assert "unexpected" not in betterlite["truncation"]
    assert betterlite["result"]["ok"] is True
    assert "relevance_score" in betterlite["result_missing"]["message"]
    assert betterlite["optional"] == [
        "documents", "instruction", "max_chunks_per_doc", "max_tokens_per_doc",
        "max_tokens_per_query", "query", "rank_fields", "return_documents", "top_n",
        "truncate_prompt_tokens", "truncation_side",
    ]


def test_embedding_names(report: Report) -> None:
    shapes = module_shapes(report, "embedding")
    request_shape = shapes["betterlite"]["EmbeddingRequest"]
    assert request_shape is not None
    assert request_shape["kind"] == "model"
    assert request_shape["required"] == ["model"]
    betterlite, litellm = module_behavior(report, "embedding", TypeAdapter(EmbeddingBehavior))
    assert betterlite == litellm
    assert betterlite["defaults"]["input"] == []
    assert betterlite["defaults"]["timeout"] == EMBEDDING_TIMEOUT
    assert betterlite["defaults"]["caching"] is False
    assert betterlite["defaults"]["api_base"] is None
    assert betterlite["extended"]["unknown"] == "kept"
    assert betterlite["extended"]["custom_llm_provider"] == {"provider": "x"}
    assert betterlite["missing"]["error"] == "ValidationError"
    assert "model" in betterlite["missing"]["message"]
    assert betterlite["bad_input"]["error"] == "ValidationError"
    assert "input.0" in betterlite["bad_input"]["message"]


def test_fine_tuning_names(report: Report) -> None:
    shapes = described_shapes(report, "fine_tuning")
    litellm_shape = shapes["litellm"]["OpenAIFineTuningHyperparameters"]
    betterlite_shape = shapes["betterlite"]["OpenAIFineTuningHyperparameters"]
    assert litellm_shape is not None
    assert betterlite_shape is not None
    assert betterlite_shape["kind"] == "model"
    without_literals = {
        field: value for field, value in litellm_shape.items() if field != "literals"
    }
    assert {
        field: value for field, value in betterlite_shape.items() if field != "literals"
    } == without_literals
    assert litellm_shape["literals"] == {
        "batch_size": ['"auto"'],
        "learning_rate_multiplier": ['"auto"'],
        "n_epochs": ['"auto"'],
    }
    # B7921's Hyperparameters allows any str here, so openai's Literal["auto"] is absent
    assert betterlite_shape["literals"] == {}
    betterlite, litellm = module_behavior(report, "fine_tuning", TypeAdapter(FineTuningBehavior))
    assert betterlite == litellm
    assert betterlite["values"]["batch_size"] == "auto"
    assert "unknown" in betterlite["values"]
    assert betterlite["values"]["n_epochs"] is None


def test_containers_names(report: Report) -> None:
    shapes = module_shapes(report, "containers")
    object_shape = shapes["betterlite"]["ContainerObject"]
    params_shape = shapes["betterlite"]["ContainerCreateRequestParams"]
    expires_shape = shapes["betterlite"]["ExpiresAfter"]
    assert object_shape is not None
    assert params_shape is not None
    assert expires_shape is not None
    assert object_shape["kind"] == "model"
    assert params_shape["kind"] == "typeddict"
    assert params_shape["keys"] == [
        "expires_after", "extra_body", "extra_headers", "file_ids", "name",
    ]
    assert params_shape["readonly"] == ["expires_after"]
    assert expires_shape["required"] == ["anchor", "minutes"]
    betterlite, litellm = module_behavior(report, "containers", TypeAdapter(ContainersBehavior))
    shared_betterlite = {
        field: value for field, value in betterlite.items() if field != "difference"
    }
    shared_litellm = {field: value for field, value in litellm.items() if field != "difference"}
    assert shared_betterlite == shared_litellm
    differences = betterlite["difference"] + litellm["difference"]
    assert [(entry["error"], entry["ok"]) for entry in differences] == [
        ("TypeError", False), ("TypeError", False), ("", True), ("TypeError", False),
    ]
    assert betterlite["container"] == betterlite["container_json"]
    assert betterlite["container"]["object"] == "container"
    assert betterlite["container"]["expires_after"] == {
        "anchor": "last_active_at", "minutes": 30,
    }
    assert "unexpected" not in betterlite["expires"]
    assert "unexpected" not in betterlite["container"]
    assert betterlite["container"]["last_active_at"] is None
    assert "last_active_at" not in betterlite["container_json_exclude_none"]
    assert betterlite["container_json_exclude_none"]["name"] == "box"
    assert set(betterlite["container_json_exclude_unset"]) == {
        "id", "object", "created_at", "status", "expires_after", "name",
    }
    assert "_hidden_params" not in betterlite["container"]
    assert betterlite["container_get"] == "fallback"
    assert betterlite["container_item"] == "c1"
    assert (
        betterlite["container_hidden_item"],
        betterlite["container_hidden_get"],
        betterlite["container_hidden_contains"],
    ) == ({"model_id": "m"}, {"model_id": "m"}, True)
    assert betterlite["container_contains"] == [True, False, False]
    assert betterlite["container_missing_item"]["error"] == "AttributeError"
    assert "unexpected" not in betterlite["container_extra"]
    assert betterlite["file"]["object"] == "container.file"
    assert betterlite["underscored"]["object"] == "container_file"
    assert betterlite["deleted"]["object"] == "container.deleted"
    assert betterlite["deleted_file"]["object"] == "container_file.deleted"
    assert betterlite["bad_object"]["error"] == "ValidationError"
    assert "object" in betterlite["bad_object"]["message"]
    assert "last_active_at" in betterlite["bad_anchor"]["message"]
    assert betterlite["hidden"] == {"model_id": "m"}


def test_google_genai_names(report: Report) -> None:
    shapes = module_shapes(report, "google_genai")
    response_shape = shapes["betterlite"]["GenerateContentResponse"]
    assert response_shape is not None
    assert response_shape["kind"] == "model"
    module_shapes(report, "google_genai_adapters")
    package = module_shapes(report, "google_genai_package")
    assert package["litellm"] == {}
    betterlite, litellm = module_behavior(report, "google_genai", TypeAdapter(GoogleBehavior))
    assert betterlite["common"] == litellm["common"]
    assert betterlite["common"]["request_dict"]["contents"] == [{"role": "user"}]
    assert betterlite["common"]["request_config"] == {"temperature": 0.5}
    assert betterlite["common"]["request_tools"] == [{"name": "x"}]
    assert betterlite["common"]["request_missing_tools"] == [None]
    assert list(betterlite["common"]["parameters_dict"]) == ["contents"]
    assert betterlite["common"]["parameters_dict"]["contents"] == []
    assert betterlite["common"]["config_dict"] == {"temperature": 0.2}
    assert betterlite["common"]["config_positional"]["error"] == "TypeError"
    assert betterlite["common"]["plain_dump"] == {}
    assert betterlite["common"]["response_hidden"] == {"model_id": "m"}
    assert betterlite["response_dump"] == {"prompt": "hi"}
    assert litellm["response_dump"] == {"prompt": "hi", "_hidden_params": {"model_id": "m"}}
    adapters_behavior, litellm_adapters = module_behavior(
        report, "google_genai_adapters", TypeAdapter(OptionalKeysBehavior),
    )
    assert adapters_behavior == litellm_adapters
    assert adapters_behavior["optional"] == [
        "extra_headers", "max_tokens", "messages", "metadata", "model",
        "proxy_server_request", "stop", "stream", "temperature", "tool_choice", "tools", "top_p",
    ]
    package_behavior, litellm_package = module_behavior(
        report, "google_genai_package", TypeAdapter(PackageBehavior),
    )
    assert package_behavior == litellm_package
    assert package_behavior["all"] == [
        "ContentListUnion", "ContentListUnionDict", "GenerateContentConfigOrDict",
        "GenerateContentResponse",
    ]


def test_images_names(report: Report) -> None:
    shapes = module_shapes(report, "images")
    optional_shape = shapes["betterlite"]["ImageEditOptionalRequestParams"]
    request_shape = shapes["betterlite"]["ImageEditRequestParams"]
    assert optional_shape is not None
    assert request_shape is not None
    assert optional_shape["kind"] == "typeddict"
    assert request_shape["kind"] == "typeddict"
    assert optional_shape["literals"]["background"] == ['"transparent"', '"opaque"', '"auto"']
    assert optional_shape["literals"]["input_fidelity"] == ['"high"', '"low"']
    assert "imageConfig" in request_shape["keys"]
    betterlite, litellm = module_behavior(report, "images", TypeAdapter(ImagesBehavior))
    assert betterlite == litellm
    assert betterlite["request"] == [
        "background", "image", "imageConfig", "input_fidelity", "mask", "model", "n",
        "prompt", "quality", "response_format", "size", "user",
    ]


def test_realtime_names(report: Report) -> None:
    shapes = module_shapes(report, "realtime")
    session_shape = shapes["betterlite"]["RealtimeSessionConfig"]
    secret_shape = shapes["betterlite"]["RealtimeClientSecretResponse"]
    error_shape = shapes["betterlite"]["RealtimeErrorEvent"]
    usage_shape = shapes["betterlite"]["RealtimeInputAudioTranscriptionTokenUsage"]
    assert session_shape is not None
    assert secret_shape is not None
    assert error_shape is not None
    assert usage_shape is not None
    assert session_shape["kind"] == "model"
    assert secret_shape["kind"] == "model"
    assert error_shape["kind"] == "typeddict"
    assert secret_shape["required"] == ["value"]
    assert usage_shape["literals"] == {"type": ['"tokens"']}
    betterlite, litellm = module_behavior(report, "realtime", TypeAdapter(RealtimeBehavior))
    assert betterlite == litellm
    assert betterlite["session"]["unknown"] == "kept"
    assert "unknown" not in betterlite["secret"]
    assert "unknown" not in betterlite["secret_response"]
    assert betterlite["transcription"]["unknown"] == "kept"
    assert betterlite["transcription_response"]["unknown"] == "kept"
    assert betterlite["secret_response"]["value"] == "ek_1"
    assert betterlite["resolved"] == [None, "m1", "m2", None, "m3"]
    assert betterlite["expires"]["anchor"] == "created_at"
    assert betterlite["expires"]["seconds"] is None
    assert "unexpected" not in betterlite["expires"]
    assert betterlite["expires_anchor"]["anchor"] is None
    assert "unexpected" not in betterlite["expires_anchor"]
    assert betterlite["missing_value"]["error"] == "ValidationError"
    assert "value" in betterlite["missing_value"]["message"]
    assert betterlite["delta_types"][0]["ok"] is True
    assert betterlite["delta_types"][1]["ok"] is True
    assert betterlite["delta_types"][2]["ok"] is False
    assert "Input should be 'text' or 'audio'" in betterlite["delta_types"][2]["message"]
    assert betterlite["error_event"] == {
        "type": "error", "error": {"type": "invalid_request", "message": "bad"},
    }
