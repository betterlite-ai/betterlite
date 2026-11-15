import operator
from os import PathLike
from typing import ClassVar, Literal, Protocol, TypeAlias, get_args, get_origin, get_type_hints

import litellm.types.llms.openai as litellm_openai  # pyright: ignore[reportMissingTypeStubs]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L1
import pytest
from pydantic import BaseModel, JsonValue, ValidationError
from typing_extensions import ReadOnly, TypeForm

import betterlite.types.llms.openai.core.batches as core_batches
import betterlite.types.llms.openai.core.binary as core_binary
import betterlite.types.llms.openai.core.chat_blocks as core_chat_blocks
import betterlite.types.llms.openai.core.files as core_files
import betterlite.types.llms.openai.core.messages as core_messages
import betterlite.types.llms.openai.core.not_given as core_not_given
import betterlite.types.llms.openai.core.threads as core_threads
from betterlite.types.llms.openai.core.aliases import (
    AllPromptValues,
    EmbeddingInput,
    FileContent,
    FileTypes,
)
from betterlite.types.llms.openai.core.chat_blocks import (
    OpenAIMessageContent,
    OpenAIMessageContentListBlock,
)
from betterlite.types.llms.openai.core.messages import AllMessageValues, ToolMessageContentPart
from betterlite.types.llms.openai.core.threads import AttachmentTool

HintMap: TypeAlias = dict[str, TypeForm[JsonValue] | None]


class TypedDictClass(Protocol):
    __required_keys__: ClassVar[frozenset[str]]
    __optional_keys__: ClassVar[frozenset[str]]


class OurTypedDicts(Protocol):
    BinaryResponseSummary: core_binary.BinaryResponseSummary
    ToolResourcesCodeInterpreter: core_threads.ToolResourcesCodeInterpreter
    # ast-grep-ignore: no-long-names
    ToolResourcesFileSearchVectorStore: core_threads.ToolResourcesFileSearchVectorStore
    ToolResourcesFileSearch: core_threads.ToolResourcesFileSearch
    # ast-grep-ignore: no-long-names
    OpenAICreateThreadParamsToolResources: core_threads.OpenAICreateThreadParamsToolResources
    FileSearchToolParam: core_threads.FileSearchToolParam
    CodeInterpreterToolParam: core_threads.CodeInterpreterToolParam
    Attachment: core_threads.Attachment
    ImageFileObject: core_threads.ImageFileObject
    ImageURLObject: core_threads.ImageURLObject
    ImageURLListItem: core_threads.ImageURLListItem
    MessageContentTextObject: core_threads.MessageContentTextObject
    MessageContentImageFileObject: core_threads.MessageContentImageFileObject
    MessageContentImageURLObject: core_threads.MessageContentImageURLObject
    MessageData: core_threads.MessageData
    FileExpiresAfter: core_files.FileExpiresAfter
    CreateFileRequest: core_files.CreateFileRequest
    FileContentRequest: core_files.FileContentRequest
    CreateBatchRequest: core_batches.CreateBatchRequest
    LiteLLMBatchCreateRequest: core_batches.LiteLLMBatchCreateRequest
    RetrieveBatchRequest: core_batches.RetrieveBatchRequest
    CancelBatchRequest: core_batches.CancelBatchRequest
    ListBatchRequest: core_batches.ListBatchRequest
    OpenAIErrorBody: core_batches.OpenAIErrorBody
    ChatCompletionAudioDelta: core_chat_blocks.ChatCompletionAudioDelta
    # ast-grep-ignore: no-long-names
    ChatCompletionToolCallFunctionChunk: core_chat_blocks.ChatCompletionToolCallFunctionChunk
    # ast-grep-ignore: no-long-names
    ChatCompletionAssistantToolCall: core_chat_blocks.ChatCompletionAssistantToolCall
    ChatCompletionToolCallChunk: core_chat_blocks.ChatCompletionToolCallChunk
    # ast-grep-ignore: no-long-names
    ChatCompletionDeltaToolCallChunk: core_chat_blocks.ChatCompletionDeltaToolCallChunk
    ChatCompletionCachedContent: core_chat_blocks.ChatCompletionCachedContent
    PromptCacheBreakpoint: core_chat_blocks.PromptCacheBreakpoint
    PromptCacheOptions: core_chat_blocks.PromptCacheOptions
    ChatCompletionThinkingBlock: core_chat_blocks.ChatCompletionThinkingBlock
    # ast-grep-ignore: no-long-names
    ChatCompletionRedactedThinkingBlock: core_chat_blocks.ChatCompletionRedactedThinkingBlock
    # ast-grep-ignore: no-long-names
    ChatCompletionReasoningSummaryTextBlock: (
        core_chat_blocks.ChatCompletionReasoningSummaryTextBlock
    )
    ChatCompletionReasoningItem: core_chat_blocks.ChatCompletionReasoningItem
    # ast-grep-ignore: no-long-names
    WebSearchOptionsUserLocationApproximate: (
        core_chat_blocks.WebSearchOptionsUserLocationApproximate
    )
    WebSearchOptionsUserLocation: core_chat_blocks.WebSearchOptionsUserLocation
    WebSearchOptions: core_chat_blocks.WebSearchOptions
    FileSearchTool: core_chat_blocks.FileSearchTool
    # ast-grep-ignore: no-long-names
    ChatCompletionAnnotationURLCitation: core_chat_blocks.ChatCompletionAnnotationURLCitation
    ChatCompletionAnnotation: core_chat_blocks.ChatCompletionAnnotation
    OpenAIChatCompletionTextObject: core_chat_blocks.OpenAIChatCompletionTextObject
    ChatCompletionTextObject: core_chat_blocks.ChatCompletionTextObject
    ChatCompletionImageUrlObject: core_chat_blocks.ChatCompletionImageUrlObject
    ChatCompletionImageObject: core_chat_blocks.ChatCompletionImageObject
    ChatCompletionVideoUrlObject: core_chat_blocks.ChatCompletionVideoUrlObject
    ChatCompletionVideoObject: core_chat_blocks.ChatCompletionVideoObject
    DocumentObject: core_chat_blocks.DocumentObject
    CitationsObject: core_chat_blocks.CitationsObject
    ChatCompletionDocumentObject: core_chat_blocks.ChatCompletionDocumentObject
    ChatCompletionFileObjectFile: core_chat_blocks.ChatCompletionFileObjectFile
    ChatCompletionFileObject: core_chat_blocks.ChatCompletionFileObject
    # ast-grep-ignore: no-long-names
    OpenAIChatCompletionUserMessage: core_messages.OpenAIChatCompletionUserMessage
    # ast-grep-ignore: no-long-names
    OpenAITextCompletionUserMessage: core_messages.OpenAITextCompletionUserMessage
    ChatCompletionUserMessage: core_messages.ChatCompletionUserMessage
    # ast-grep-ignore: no-long-names
    OpenAIChatCompletionAssistantMessage: core_messages.OpenAIChatCompletionAssistantMessage
    ChatCompletionAssistantMessage: core_messages.ChatCompletionAssistantMessage
    # ast-grep-ignore: no-long-names
    ChatCompletionToolReferenceObject: core_messages.ChatCompletionToolReferenceObject
    ChatCompletionToolMessage: core_messages.ChatCompletionToolMessage
    ChatCompletionFunctionMessage: core_messages.ChatCompletionFunctionMessage
    # ast-grep-ignore: no-long-names
    OpenAIChatCompletionSystemMessage: core_messages.OpenAIChatCompletionSystemMessage
    # ast-grep-ignore: no-long-names
    OpenAIChatCompletionDeveloperMessage: core_messages.OpenAIChatCompletionDeveloperMessage
    ChatCompletionSystemMessage: core_messages.ChatCompletionSystemMessage
    ChatCompletionDeveloperMessage: core_messages.ChatCompletionDeveloperMessage
    GenericChatCompletionMessage: core_messages.GenericChatCompletionMessage


class LiteLLMTypedDicts(Protocol):
    BinaryResponseSummary: litellm_openai.BinaryResponseSummary
    ToolResourcesCodeInterpreter: litellm_openai.ToolResourcesCodeInterpreter
    # ast-grep-ignore: no-long-names
    ToolResourcesFileSearchVectorStore: litellm_openai.ToolResourcesFileSearchVectorStore
    ToolResourcesFileSearch: litellm_openai.ToolResourcesFileSearch
    # ast-grep-ignore: no-long-names
    OpenAICreateThreadParamsToolResources: litellm_openai.OpenAICreateThreadParamsToolResources
    FileSearchToolParam: litellm_openai.FileSearchToolParam
    CodeInterpreterToolParam: litellm_openai.CodeInterpreterToolParam
    Attachment: litellm_openai.Attachment
    ImageFileObject: litellm_openai.ImageFileObject
    ImageURLObject: litellm_openai.ImageURLObject
    ImageURLListItem: litellm_openai.ImageURLListItem
    MessageContentTextObject: litellm_openai.MessageContentTextObject
    MessageContentImageFileObject: litellm_openai.MessageContentImageFileObject
    MessageContentImageURLObject: litellm_openai.MessageContentImageURLObject
    MessageData: litellm_openai.MessageData
    FileExpiresAfter: litellm_openai.FileExpiresAfter
    CreateFileRequest: litellm_openai.CreateFileRequest
    FileContentRequest: litellm_openai.FileContentRequest
    CreateBatchRequest: litellm_openai.CreateBatchRequest
    LiteLLMBatchCreateRequest: litellm_openai.LiteLLMBatchCreateRequest
    RetrieveBatchRequest: litellm_openai.RetrieveBatchRequest
    CancelBatchRequest: litellm_openai.CancelBatchRequest
    ListBatchRequest: litellm_openai.ListBatchRequest
    OpenAIErrorBody: litellm_openai.OpenAIErrorBody
    ChatCompletionAudioDelta: litellm_openai.ChatCompletionAudioDelta
    # ast-grep-ignore: no-long-names
    ChatCompletionToolCallFunctionChunk: litellm_openai.ChatCompletionToolCallFunctionChunk
    # ast-grep-ignore: no-long-names
    ChatCompletionAssistantToolCall: litellm_openai.ChatCompletionAssistantToolCall
    ChatCompletionToolCallChunk: litellm_openai.ChatCompletionToolCallChunk
    # ast-grep-ignore: no-long-names
    ChatCompletionDeltaToolCallChunk: litellm_openai.ChatCompletionDeltaToolCallChunk
    ChatCompletionCachedContent: litellm_openai.ChatCompletionCachedContent
    PromptCacheBreakpoint: litellm_openai.PromptCacheBreakpoint
    PromptCacheOptions: litellm_openai.PromptCacheOptions
    ChatCompletionThinkingBlock: litellm_openai.ChatCompletionThinkingBlock
    # ast-grep-ignore: no-long-names
    ChatCompletionRedactedThinkingBlock: litellm_openai.ChatCompletionRedactedThinkingBlock
    # ast-grep-ignore: no-long-names
    ChatCompletionReasoningSummaryTextBlock: litellm_openai.ChatCompletionReasoningSummaryTextBlock
    ChatCompletionReasoningItem: litellm_openai.ChatCompletionReasoningItem
    # ast-grep-ignore: no-long-names
    WebSearchOptionsUserLocationApproximate: litellm_openai.WebSearchOptionsUserLocationApproximate
    WebSearchOptionsUserLocation: litellm_openai.WebSearchOptionsUserLocation
    WebSearchOptions: litellm_openai.WebSearchOptions
    FileSearchTool: litellm_openai.FileSearchTool
    # ast-grep-ignore: no-long-names
    ChatCompletionAnnotationURLCitation: litellm_openai.ChatCompletionAnnotationURLCitation
    ChatCompletionAnnotation: litellm_openai.ChatCompletionAnnotation
    OpenAIChatCompletionTextObject: litellm_openai.OpenAIChatCompletionTextObject
    ChatCompletionTextObject: litellm_openai.ChatCompletionTextObject
    ChatCompletionImageUrlObject: litellm_openai.ChatCompletionImageUrlObject
    ChatCompletionImageObject: litellm_openai.ChatCompletionImageObject
    ChatCompletionVideoUrlObject: litellm_openai.ChatCompletionVideoUrlObject
    ChatCompletionVideoObject: litellm_openai.ChatCompletionVideoObject
    DocumentObject: litellm_openai.DocumentObject
    CitationsObject: litellm_openai.CitationsObject
    ChatCompletionDocumentObject: litellm_openai.ChatCompletionDocumentObject
    ChatCompletionFileObjectFile: litellm_openai.ChatCompletionFileObjectFile
    ChatCompletionFileObject: litellm_openai.ChatCompletionFileObject
    # ast-grep-ignore: no-long-names
    OpenAIChatCompletionUserMessage: litellm_openai.OpenAIChatCompletionUserMessage
    # ast-grep-ignore: no-long-names
    OpenAITextCompletionUserMessage: litellm_openai.OpenAITextCompletionUserMessage
    ChatCompletionUserMessage: litellm_openai.ChatCompletionUserMessage
    # ast-grep-ignore: no-long-names
    OpenAIChatCompletionAssistantMessage: litellm_openai.OpenAIChatCompletionAssistantMessage
    ChatCompletionAssistantMessage: litellm_openai.ChatCompletionAssistantMessage
    # ast-grep-ignore: no-long-names
    ChatCompletionToolReferenceObject: litellm_openai.ChatCompletionToolReferenceObject
    ChatCompletionToolMessage: litellm_openai.ChatCompletionToolMessage
    ChatCompletionFunctionMessage: litellm_openai.ChatCompletionFunctionMessage
    # ast-grep-ignore: no-long-names
    OpenAIChatCompletionSystemMessage: litellm_openai.OpenAIChatCompletionSystemMessage
    # ast-grep-ignore: no-long-names
    OpenAIChatCompletionDeveloperMessage: litellm_openai.OpenAIChatCompletionDeveloperMessage
    ChatCompletionSystemMessage: litellm_openai.ChatCompletionSystemMessage
    ChatCompletionDeveloperMessage: litellm_openai.ChatCompletionDeveloperMessage
    GenericChatCompletionMessage: litellm_openai.GenericChatCompletionMessage


class OurFileTypes(Protocol):
    file_types: FileTypes


class LiteLLMFileTypes(Protocol):
    file_types: litellm_openai.FileTypes


def literal_values(annotation: TypeForm[JsonValue] | None) -> tuple[str, ...]:
    values: list[str] = []
    pending: list[TypeForm[JsonValue] | None] = [annotation]
    while pending:
        current = pending.pop()
        origin: TypeForm[JsonValue] | None = get_origin(current)
        # a plain is check against Literal trips mypy's non-overlapping identity check
        if operator.is_(origin, Literal):
            arguments: tuple[TypeForm[JsonValue] | None, ...] = get_args(current)
            values.extend(str(argument) for argument in arguments)
            continue
        nested: tuple[TypeForm[JsonValue] | None, ...] = get_args(current)
        pending.extend(nested)
    return tuple(values)


def check_file_content_forms(
    ours: tuple[TypeForm[JsonValue], ...], theirs: tuple[TypeForm[JsonValue], ...]
) -> None:
    assert len(ours) == len(theirs) + 1
    assert ours[:2] == theirs[:2]
    ours_text_origin: TypeForm[JsonValue] | None = get_origin(ours[2])
    ours_text_parameters: tuple[TypeForm[JsonValue], ...] = get_args(ours[2])
    ours_bytes_origin: TypeForm[JsonValue] | None = get_origin(ours[3])
    ours_bytes_parameters: tuple[TypeForm[JsonValue], ...] = get_args(ours[3])
    assert operator.is_(ours_text_origin, PathLike)
    assert len(ours_text_parameters) == 1
    assert operator.is_(ours_text_parameters[0], str)
    assert operator.is_(ours_bytes_origin, PathLike)
    assert len(ours_bytes_parameters) == 1
    assert operator.is_(ours_bytes_parameters[0], bytes)
    assert operator.is_(theirs[2], PathLike)


def check_annotations_equal(
    ours: TypeForm[JsonValue] | None, theirs: TypeForm[JsonValue] | None
) -> None:
    ours_origin: TypeForm[JsonValue] | None = get_origin(ours)
    theirs_origin: TypeForm[JsonValue] | None = get_origin(theirs)
    assert operator.is_(ours_origin, ReadOnly) == operator.is_(theirs_origin, ReadOnly)
    assert literal_values(ours) == literal_values(theirs)


def check_hint_maps(ours: HintMap, theirs: HintMap) -> None:
    assert ours.keys() == theirs.keys()
    for key in ours.keys() & theirs.keys():
        check_annotations_equal(ours[key], theirs[key])


def model_hints(model: type[BaseModel]) -> HintMap:
    hints: HintMap = {}
    for key, field in model.model_fields.items():
        annotation: TypeForm[JsonValue] | None = field.annotation
        hints[key] = annotation
    return hints


def check_model_literals(ours: type[BaseModel], theirs: type[BaseModel]) -> None:
    assert ours.model_fields.keys() == theirs.model_fields.keys()
    check_hint_maps(model_hints(ours), model_hints(theirs))


def check_typed_dict(ours: type[TypedDictClass], theirs: type[TypedDictClass]) -> None:
    assert ours.__required_keys__ == theirs.__required_keys__
    assert ours.__optional_keys__ == theirs.__optional_keys__
    ours_hints: HintMap = get_type_hints(ours, include_extras=True)
    theirs_hints: HintMap = get_type_hints(theirs, include_extras=True)
    check_hint_maps(ours_hints, theirs_hints)


def test_names_typed_dicts() -> None:
    ours: dict[str, type[TypedDictClass]] = get_type_hints(OurTypedDicts)
    theirs: dict[str, type[TypedDictClass]] = get_type_hints(LiteLLMTypedDicts)
    assert ours.keys() == theirs.keys()
    for name in ours.keys() & theirs.keys():
        check_typed_dict(ours[name], theirs[name])


def test_names_literal_aliases() -> None:
    assert core_batches.BatchJobStatus == litellm_openai.BatchJobStatus
    assert core_files.CREATE_FILE_REQUESTS_PURPOSE == litellm_openai.CREATE_FILE_REQUESTS_PURPOSE
    assert core_files.OpenAIFilesPurpose == litellm_openai.OpenAIFilesPurpose
    assert (
        core_files.OpenAICreateFileRequestOptionalParams
        == litellm_openai.OpenAICreateFileRequestOptionalParams
    )
    assert (
        core_messages.ValidUserMessageContentTypesLiteral
        == litellm_openai.ValidUserMessageContentTypesLiteral
    )
    assert (
        core_messages.ValidAssistantMessageContentTypesLiteral
        == litellm_openai.ValidAssistantMessageContentTypesLiteral
    )
    assert (
        core_messages.ValidChatCompletionMessageContentTypesLiteral
        == litellm_openai.ValidChatCompletionMessageContentTypesLiteral
    )


def test_names_model_literals() -> None:
    check_model_literals(core_files.OpenAIFileObject, litellm_openai.OpenAIFileObject)
    check_model_literals(core_files.BatchGuardrailRecord, litellm_openai.BatchGuardrailRecord)
    check_model_literals(core_files.FileListPage, litellm_openai.FileListPage)


def test_names_audio_object_keys() -> None:
    ours = core_chat_blocks.ChatCompletionAudioObject
    theirs = litellm_openai.ChatCompletionAudioObject
    assert sorted(ours.__required_keys__) == ["input_audio", "type"]
    assert sorted(ours.__optional_keys__) == ["prompt_cache_breakpoint"]
    assert ours.__required_keys__ | ours.__optional_keys__ == (
        theirs.__required_keys__ | theirs.__optional_keys__
    )
    assert not theirs.__required_keys__
    assert sorted(theirs.__optional_keys__) == [
        "input_audio",
        "prompt_cache_breakpoint",
        "type",
    ]


def test_names_shared_aliases() -> None:
    _content: FileContent = b"raw"
    _file_types: FileTypes = ("a.txt", b"raw")
    _embedding: EmbeddingInput = ["a"]
    _prompt: AllPromptValues = ["a", "b"]
    _message_content: OpenAIMessageContent = "hi"
    text_object = core_chat_blocks.ChatCompletionTextObject(type="text", text="hi")
    _block: OpenAIMessageContentListBlock = text_object
    search_tool = core_threads.FileSearchToolParam(type="file_search")
    _attachment_tool: AttachmentTool = search_tool
    tool_reference = core_messages.ChatCompletionToolReferenceObject(
        type="tool_reference", tool_name="t"
    )
    _tool_part: ToolMessageContentPart = tool_reference
    user_message = core_messages.ChatCompletionUserMessage(role="user", content="hi")
    _message: AllMessageValues = user_message
    assert EmbeddingInput == litellm_openai.EmbeddingInput
    assert AllPromptValues == litellm_openai.AllPromptValues
    ours_hints: HintMap = get_type_hints(OurFileTypes)
    theirs_hints: HintMap = get_type_hints(LiteLLMFileTypes)
    ours_members: tuple[TypeForm[JsonValue], ...] = get_args(ours_hints["file_types"])
    theirs_members: tuple[TypeForm[JsonValue], ...] = get_args(theirs_hints["file_types"])
    assert len(ours_members) == len(theirs_members) + 1
    check_file_content_forms(ours_members[:4], theirs_members[:3])
    for ours_tuple, theirs_tuple in zip(ours_members[4:], theirs_members[3:], strict=True):
        ours_slots: tuple[TypeForm[JsonValue], ...] = get_args(ours_tuple)
        theirs_slots: tuple[TypeForm[JsonValue], ...] = get_args(theirs_tuple)
        assert len(ours_slots) == len(theirs_slots)
        assert ours_slots[0] == theirs_slots[0]
        assert ours_slots[2:] == theirs_slots[2:]
        ours_content: tuple[TypeForm[JsonValue], ...] = get_args(ours_slots[1])
        theirs_content: tuple[TypeForm[JsonValue], ...] = get_args(theirs_slots[1])
        check_file_content_forms(ours_content, theirs_content)


def test_models_file_and_page_dump() -> None:
    ours = core_files.OpenAIFileObject(
        id="file_1",
        bytes=7,
        created_at=10,
        filename="a.jsonl",
        object="file",
        purpose="batch",
        status="processed",
        expires_at=20,
        status_details="ok",
    )
    theirs = litellm_openai.OpenAIFileObject(
        id="file_1",
        bytes=7,
        created_at=10,
        filename="a.jsonl",
        object="file",
        purpose="batch",
        status="processed",
        expires_at=20,
        status_details="ok",
    )
    theirs_dump: dict[str, JsonValue] = theirs.model_dump()
    assert ours.model_dump() == theirs_dump
    assert ours.json() == ours.model_dump()
    defaults = core_files.OpenAIFileObject(
        id="file_1",
        bytes=7,
        created_at=10,
        filename="a.jsonl",
        object="file",
        purpose="assistants",
    )
    reference_defaults = litellm_openai.OpenAIFileObject(
        id="file_1",
        bytes=7,
        created_at=10,
        filename="a.jsonl",
        object="file",
        purpose="assistants",
    )
    reference_defaults_dump: dict[str, JsonValue] = reference_defaults.model_dump()
    assert defaults.model_dump() == reference_defaults_dump
    page = core_files.FileListPage()
    reference_page = litellm_openai.FileListPage()
    reference_page_dump: dict[str, JsonValue] = reference_page.model_dump()
    assert page.model_dump() == reference_page_dump
    filled_page = core_files.FileListPage(first_id="a", last_id="b", has_more=True)
    reference_filled_page = litellm_openai.FileListPage(
        first_id="a", last_id="b", has_more=True
    )
    reference_filled_page_dump: dict[str, JsonValue] = reference_filled_page.model_dump()
    assert filled_page.model_dump() == reference_filled_page_dump
    extra_payload: dict[str, JsonValue] = {
        "id": "file_1",
        "bytes": 7,
        "created_at": 10,
        "filename": "a.jsonl",
        "object": "file",
        "purpose": "batch",
        "bogus_field": 1,
    }
    extra = core_files.OpenAIFileObject.model_validate(extra_payload)
    reference_extra = litellm_openai.OpenAIFileObject.model_validate(extra_payload)
    assert "bogus_field" in extra.model_dump()
    reference_extra_dump: dict[str, JsonValue] = reference_extra.model_dump()
    assert "bogus_field" not in reference_extra_dump


# ast-grep-ignore: no-long-names
def test_models_file_object_guardrail_field() -> None:
    absent = core_files.OpenAIFileObject(
        id="file_1",
        bytes=7,
        created_at=10,
        filename="a.jsonl",
        object="file",
        purpose="batch",
        litellm_batch_guardrail=None,
    )
    reference_absent = litellm_openai.OpenAIFileObject(
        id="file_1",
        bytes=7,
        created_at=10,
        filename="a.jsonl",
        object="file",
        purpose="batch",
        litellm_batch_guardrail=None,
    )
    field = core_files.BATCH_GUARDRAIL_RESPONSE_FIELD
    assert field not in absent.model_dump()
    assert field not in absent.model_dump_json()
    reference_absent_dump: dict[str, JsonValue] = reference_absent.model_dump()
    assert absent.model_dump() == reference_absent_dump
    ours_record = core_files.BatchGuardrailRecord(
        line=1, custom_id="c1", action="redacted", guardrail=None,
    )
    ours_report = core_files.BatchGuardrailReport(
        submitted_records=1, modified_records=(ours_record,),
    )
    theirs_record = litellm_openai.BatchGuardrailRecord(
        line=1, custom_id="c1", action="redacted", guardrail=None,
    )
    theirs_report = litellm_openai.BatchGuardrailReport(
        submitted_records=1, modified_records=(theirs_record,),
    )
    theirs_record_dump: dict[str, JsonValue] = theirs_record.model_dump()
    theirs_report_dump: dict[str, JsonValue] = theirs_report.model_dump()
    assert ours_record.model_dump() == theirs_record_dump
    assert ours_report.model_dump() == theirs_report_dump
    present = core_files.OpenAIFileObject(
        id="file_1",
        bytes=7,
        created_at=10,
        filename="a.jsonl",
        object="file",
        purpose="batch",
        litellm_batch_guardrail=ours_report,
    )
    reference_present = litellm_openai.OpenAIFileObject(
        id="file_1",
        bytes=7,
        created_at=10,
        filename="a.jsonl",
        object="file",
        purpose="batch",
        litellm_batch_guardrail=theirs_report,
    )
    assert field in present.model_dump()
    reference_present_dump: dict[str, JsonValue] = reference_present.model_dump()
    assert present.model_dump() == reference_present_dump
    page = core_files.FileListPage(data=[present])
    reference_page = litellm_openai.FileListPage(data=[reference_present])
    reference_page_dump: dict[str, JsonValue] = reference_page.model_dump()
    assert page.model_dump() == reference_page_dump


def test_models_thread_dump() -> None:
    ours = core_threads.Thread(id="thread_1", created_at=10, object="thread")
    theirs = litellm_openai.Thread(id="thread_1", created_at=10, object="thread")
    theirs_dump: dict[str, JsonValue] = theirs.model_dump()
    assert ours.model_dump() == theirs_dump
    ours_metadata = core_threads.Thread(
        id="thread_1", created_at=10, object="thread", metadata={"key": "value"}
    )
    theirs_metadata = litellm_openai.Thread(
        id="thread_1", created_at=10, object="thread", metadata={"key": "value"}
    )
    theirs_metadata_dump: dict[str, JsonValue] = theirs_metadata.model_dump()
    assert ours_metadata.model_dump() == theirs_metadata_dump


# ast-grep-ignore: no-long-names
def test_models_file_object_rejects_unknown_purpose() -> None:
    payload: dict[str, JsonValue] = {
        "id": "file_1",
        "bytes": 7,
        "created_at": 10,
        "filename": "a.jsonl",
        "object": "file",
        "purpose": "not-a-purpose",
    }
    with pytest.raises(ValidationError):
        core_files.OpenAIFileObject.model_validate(payload)


def test_constants_not_given() -> None:
    assert not core_not_given.NOT_GIVEN
    assert repr(core_not_given.NOT_GIVEN) == "NOT_GIVEN"
    assert not core_not_given.NotGiven()
    assert repr(core_not_given.NotGiven()) == "NOT_GIVEN"
    assert sorted(core_batches.ListBatchRequest.__optional_keys__) == ["after"]


# ast-grep-ignore: no-long-names
def test_constants_content_type_lists() -> None:
    ours_user = core_messages.ValidUserMessageContentTypes
    theirs_user = litellm_openai.ValidUserMessageContentTypes
    assert ours_user == theirs_user
    ours_assistant = core_messages.ValidAssistantMessageContentTypes
    theirs_assistant = litellm_openai.ValidAssistantMessageContentTypes
    assert ours_assistant == theirs_assistant
    ours_chat = core_messages.ValidChatCompletionMessageContentTypes
    theirs_chat = litellm_openai.ValidChatCompletionMessageContentTypes
    assert ours_chat == theirs_chat
