import litellm.types.llms.openai as litellm_openai
import pytest
from pydantic import JsonValue, ValidationError

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


def test_names_typed_dicts() -> None:
    typed_dicts = (
        (
            core_binary.BinaryResponseSummary,
            litellm_openai.BinaryResponseSummary,
        ),
        (
            core_threads.ToolResourcesCodeInterpreter,
            litellm_openai.ToolResourcesCodeInterpreter,
        ),
        (
            core_threads.ToolResourcesFileSearchVectorStore,
            litellm_openai.ToolResourcesFileSearchVectorStore,
        ),
        (
            core_threads.ToolResourcesFileSearch,
            litellm_openai.ToolResourcesFileSearch,
        ),
        (
            core_threads.OpenAICreateThreadParamsToolResources,
            litellm_openai.OpenAICreateThreadParamsToolResources,
        ),
        (
            core_threads.FileSearchToolParam,
            litellm_openai.FileSearchToolParam,
        ),
        (
            core_threads.CodeInterpreterToolParam,
            litellm_openai.CodeInterpreterToolParam,
        ),
        (
            core_threads.Attachment,
            litellm_openai.Attachment,
        ),
        (
            core_threads.ImageFileObject,
            litellm_openai.ImageFileObject,
        ),
        (
            core_threads.ImageURLObject,
            litellm_openai.ImageURLObject,
        ),
        (
            core_threads.ImageURLListItem,
            litellm_openai.ImageURLListItem,
        ),
        (
            core_threads.MessageContentTextObject,
            litellm_openai.MessageContentTextObject,
        ),
        (
            core_threads.MessageContentImageFileObject,
            litellm_openai.MessageContentImageFileObject,
        ),
        (
            core_threads.MessageContentImageURLObject,
            litellm_openai.MessageContentImageURLObject,
        ),
        (
            core_threads.MessageData,
            litellm_openai.MessageData,
        ),
        (
            core_files.FileExpiresAfter,
            litellm_openai.FileExpiresAfter,
        ),
        (
            core_files.CreateFileRequest,
            litellm_openai.CreateFileRequest,
        ),
        (
            core_files.FileContentRequest,
            litellm_openai.FileContentRequest,
        ),
        (
            core_batches.CreateBatchRequest,
            litellm_openai.CreateBatchRequest,
        ),
        (
            core_batches.LiteLLMBatchCreateRequest,
            litellm_openai.LiteLLMBatchCreateRequest,
        ),
        (
            core_batches.RetrieveBatchRequest,
            litellm_openai.RetrieveBatchRequest,
        ),
        (
            core_batches.CancelBatchRequest,
            litellm_openai.CancelBatchRequest,
        ),
        (
            core_batches.ListBatchRequest,
            litellm_openai.ListBatchRequest,
        ),
        (
            core_batches.OpenAIErrorBody,
            litellm_openai.OpenAIErrorBody,
        ),
        (
            core_chat_blocks.ChatCompletionAudioDelta,
            litellm_openai.ChatCompletionAudioDelta,
        ),
        (
            core_chat_blocks.ChatCompletionToolCallFunctionChunk,
            litellm_openai.ChatCompletionToolCallFunctionChunk,
        ),
        (
            core_chat_blocks.ChatCompletionAssistantToolCall,
            litellm_openai.ChatCompletionAssistantToolCall,
        ),
        (
            core_chat_blocks.ChatCompletionToolCallChunk,
            litellm_openai.ChatCompletionToolCallChunk,
        ),
        (
            core_chat_blocks.ChatCompletionDeltaToolCallChunk,
            litellm_openai.ChatCompletionDeltaToolCallChunk,
        ),
        (
            core_chat_blocks.ChatCompletionCachedContent,
            litellm_openai.ChatCompletionCachedContent,
        ),
        (
            core_chat_blocks.PromptCacheBreakpoint,
            litellm_openai.PromptCacheBreakpoint,
        ),
        (
            core_chat_blocks.PromptCacheOptions,
            litellm_openai.PromptCacheOptions,
        ),
        (
            core_chat_blocks.ChatCompletionThinkingBlock,
            litellm_openai.ChatCompletionThinkingBlock,
        ),
        (
            core_chat_blocks.ChatCompletionRedactedThinkingBlock,
            litellm_openai.ChatCompletionRedactedThinkingBlock,
        ),
        (
            core_chat_blocks.ChatCompletionReasoningSummaryTextBlock,
            litellm_openai.ChatCompletionReasoningSummaryTextBlock,
        ),
        (
            core_chat_blocks.ChatCompletionReasoningItem,
            litellm_openai.ChatCompletionReasoningItem,
        ),
        (
            core_chat_blocks.WebSearchOptionsUserLocationApproximate,
            litellm_openai.WebSearchOptionsUserLocationApproximate,
        ),
        (
            core_chat_blocks.WebSearchOptionsUserLocation,
            litellm_openai.WebSearchOptionsUserLocation,
        ),
        (
            core_chat_blocks.WebSearchOptions,
            litellm_openai.WebSearchOptions,
        ),
        (
            core_chat_blocks.FileSearchTool,
            litellm_openai.FileSearchTool,
        ),
        (
            core_chat_blocks.ChatCompletionAnnotationURLCitation,
            litellm_openai.ChatCompletionAnnotationURLCitation,
        ),
        (
            core_chat_blocks.ChatCompletionAnnotation,
            litellm_openai.ChatCompletionAnnotation,
        ),
        (
            core_chat_blocks.OpenAIChatCompletionTextObject,
            litellm_openai.OpenAIChatCompletionTextObject,
        ),
        (
            core_chat_blocks.ChatCompletionTextObject,
            litellm_openai.ChatCompletionTextObject,
        ),
        (
            core_chat_blocks.ChatCompletionImageUrlObject,
            litellm_openai.ChatCompletionImageUrlObject,
        ),
        (
            core_chat_blocks.ChatCompletionImageObject,
            litellm_openai.ChatCompletionImageObject,
        ),
        (
            core_chat_blocks.ChatCompletionVideoUrlObject,
            litellm_openai.ChatCompletionVideoUrlObject,
        ),
        (
            core_chat_blocks.ChatCompletionVideoObject,
            litellm_openai.ChatCompletionVideoObject,
        ),
        (
            core_chat_blocks.DocumentObject,
            litellm_openai.DocumentObject,
        ),
        (
            core_chat_blocks.CitationsObject,
            litellm_openai.CitationsObject,
        ),
        (
            core_chat_blocks.ChatCompletionDocumentObject,
            litellm_openai.ChatCompletionDocumentObject,
        ),
        (
            core_chat_blocks.ChatCompletionFileObjectFile,
            litellm_openai.ChatCompletionFileObjectFile,
        ),
        (
            core_chat_blocks.ChatCompletionFileObject,
            litellm_openai.ChatCompletionFileObject,
        ),
        (
            core_messages.OpenAIChatCompletionUserMessage,
            litellm_openai.OpenAIChatCompletionUserMessage,
        ),
        (
            core_messages.OpenAITextCompletionUserMessage,
            litellm_openai.OpenAITextCompletionUserMessage,
        ),
        (
            core_messages.ChatCompletionUserMessage,
            litellm_openai.ChatCompletionUserMessage,
        ),
        (
            core_messages.OpenAIChatCompletionAssistantMessage,
            litellm_openai.OpenAIChatCompletionAssistantMessage,
        ),
        (
            core_messages.ChatCompletionAssistantMessage,
            litellm_openai.ChatCompletionAssistantMessage,
        ),
        (
            core_messages.ChatCompletionToolReferenceObject,
            litellm_openai.ChatCompletionToolReferenceObject,
        ),
        (
            core_messages.ChatCompletionToolMessage,
            litellm_openai.ChatCompletionToolMessage,
        ),
        (
            core_messages.ChatCompletionFunctionMessage,
            litellm_openai.ChatCompletionFunctionMessage,
        ),
        (
            core_messages.OpenAIChatCompletionSystemMessage,
            litellm_openai.OpenAIChatCompletionSystemMessage,
        ),
        (
            core_messages.OpenAIChatCompletionDeveloperMessage,
            litellm_openai.OpenAIChatCompletionDeveloperMessage,
        ),
        (
            core_messages.ChatCompletionSystemMessage,
            litellm_openai.ChatCompletionSystemMessage,
        ),
        (
            core_messages.ChatCompletionDeveloperMessage,
            litellm_openai.ChatCompletionDeveloperMessage,
        ),
        (
            core_messages.GenericChatCompletionMessage,
            litellm_openai.GenericChatCompletionMessage,
        ),
    )
    for pair in typed_dicts:
        assert pair[0].__required_keys__ == pair[1].__required_keys__
        assert pair[0].__optional_keys__ == pair[1].__optional_keys__
    literal_aliases = (
        (
            core_batches.BatchJobStatus,
            litellm_openai.BatchJobStatus,
        ),
        (
            core_files.CREATE_FILE_REQUESTS_PURPOSE,
            litellm_openai.CREATE_FILE_REQUESTS_PURPOSE,
        ),
        (
            core_files.OpenAIFilesPurpose,
            litellm_openai.OpenAIFilesPurpose,
        ),
        (
            core_files.OpenAICreateFileRequestOptionalParams,
            litellm_openai.OpenAICreateFileRequestOptionalParams,
        ),
        (
            core_messages.ValidUserMessageContentTypesLiteral,
            litellm_openai.ValidUserMessageContentTypesLiteral,
        ),
        (
            core_messages.ValidAssistantMessageContentTypesLiteral,
            litellm_openai.ValidAssistantMessageContentTypesLiteral,
        ),
        (
            core_messages.ValidChatCompletionMessageContentTypesLiteral,
            litellm_openai.ValidChatCompletionMessageContentTypesLiteral,
        ),
    )
    for pair in literal_aliases:
        assert pair[0] == pair[1]


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
    content: FileContent = b"raw"
    file_types: FileTypes = ("a.txt", b"raw")
    embedding: EmbeddingInput = ["a"]
    prompt: AllPromptValues = ["a", "b"]
    message_content: OpenAIMessageContent = "hi"
    text_object = core_chat_blocks.ChatCompletionTextObject(type="text", text="hi")
    block: OpenAIMessageContentListBlock = text_object
    search_tool = core_threads.FileSearchToolParam(type="file_search")
    attachment_tool: AttachmentTool = search_tool
    tool_reference = core_messages.ChatCompletionToolReferenceObject(
        type="tool_reference", tool_name="t"
    )
    tool_part: ToolMessageContentPart = tool_reference
    user_message = core_messages.ChatCompletionUserMessage(role="user", content="hi")
    message: AllMessageValues = user_message
    assert content == b"raw"
    assert file_types == ("a.txt", b"raw")
    assert embedding == ["a"]
    assert prompt == ["a", "b"]
    assert message_content == "hi"
    assert block == text_object
    assert attachment_tool == search_tool
    assert tool_part == tool_reference
    assert message == user_message


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
    assert ours.model_dump() == theirs.model_dump()
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
    assert defaults.model_dump() == reference_defaults.model_dump()
    page = core_files.FileListPage()
    reference_page = litellm_openai.FileListPage()
    assert page.model_dump() == reference_page.model_dump()
    filled_page = core_files.FileListPage(first_id="a", last_id="b", has_more=True)
    reference_filled_page = litellm_openai.FileListPage(
        first_id="a", last_id="b", has_more=True
    )
    assert filled_page.model_dump() == reference_filled_page.model_dump()
    extra_payload: dict[str, JsonValue] = {
        "id": "file_1",
        "bytes": 7,
        "created_at": 10,
        "filename": "a.jsonl",
        "object": "file",
        "purpose": "batch",
        "bogus_field": 1,
    }
    extra = core_files.OpenAIFileObject(**extra_payload)
    reference_extra = litellm_openai.OpenAIFileObject(**extra_payload)
    assert "bogus_field" in extra.model_dump()
    assert "bogus_field" not in reference_extra.model_dump()


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
    assert absent.model_dump() == reference_absent.model_dump()
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
    assert ours_record.model_dump() == theirs_record.model_dump()
    assert ours_report.model_dump() == theirs_report.model_dump()
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
    assert present.model_dump() == reference_present.model_dump()
    page = core_files.FileListPage(data=[present])
    reference_page = litellm_openai.FileListPage(data=[reference_present])
    assert page.model_dump() == reference_page.model_dump()


def test_models_thread_dump() -> None:
    ours = core_threads.Thread(id="thread_1", created_at=10, object="thread")
    theirs = litellm_openai.Thread(id="thread_1", created_at=10, object="thread")
    assert ours.model_dump() == theirs.model_dump()
    ours_metadata = core_threads.Thread(
        id="thread_1", created_at=10, object="thread", metadata={"key": "value"}
    )
    theirs_metadata = litellm_openai.Thread(
        id="thread_1", created_at=10, object="thread", metadata={"key": "value"}
    )
    assert ours_metadata.model_dump() == theirs_metadata.model_dump()


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
