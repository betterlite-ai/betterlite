#![cfg(test)]

use rstest::rstest;

use super::{append, attachment, beta, inputs, list};

#[derive(Clone, Copy)]
enum Scenario {
    AppendMessage,
    ListMessagesNoQuery,
    AttachmentTools,
    InvalidAttachmentDrop,
    InvalidAttachmentError,
    AssistantsBetaDefault,
    AzureMessageClientInputs,
}

// One case per oracle file, each driving the production operation over the fake SDK transport.
#[rstest]
#[case::append_message(Scenario::AppendMessage)]
#[case::list_messages_no_query(Scenario::ListMessagesNoQuery)]
#[case::attachment_tools(Scenario::AttachmentTools)]
#[case::invalid_attachment_drop(Scenario::InvalidAttachmentDrop)]
#[case::invalid_attachment_error(Scenario::InvalidAttachmentError)]
#[case::assistants_beta_default(Scenario::AssistantsBetaDefault)]
#[case::azure_message_client_inputs(Scenario::AzureMessageClientInputs)]
#[tokio::test]
async fn azure_message_attachments_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::AppendMessage => append::check().await,
        Scenario::ListMessagesNoQuery => list::check().await,
        Scenario::AttachmentTools => attachment::tools(),
        Scenario::InvalidAttachmentDrop => attachment::invalid_drop(),
        Scenario::InvalidAttachmentError => attachment::invalid_error(),
        Scenario::AssistantsBetaDefault => beta::check().await,
        Scenario::AzureMessageClientInputs => inputs::check().await,
    }
}
