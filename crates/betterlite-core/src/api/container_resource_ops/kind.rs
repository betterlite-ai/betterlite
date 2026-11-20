use betterlite_models::decisions::Json;
use betterlite_openai::api::container_contracts::records::{
    ContainerObject, DeleteContainerResult,
};

use super::error::ContainerOpError;
use super::mock::{DELETE_TYPE, RETRIEVE_TYPE, mock_reply};

pub(crate) const RETRIEVE_CALL_TYPE: &str = "retrieve_container";
pub(crate) const DELETE_CALL_TYPE: &str = "delete_container";

// what retrieve and delete do differently: the record, the call type, the mock and the handler entry point
pub(crate) trait Kind {
    type Reply;
    const CALL_TYPE: &'static str;
    fn mock(mock: Option<&Json>) -> Result<Option<Self::Reply>, ContainerOpError>;
    fn id(reply: &Self::Reply) -> &str;
    fn replace_id(reply: &mut Self::Reply, id: String);
}

pub(crate) struct Retrieve;

impl Kind for Retrieve {
    type Reply = ContainerObject;
    const CALL_TYPE: &'static str = RETRIEVE_CALL_TYPE;

    fn mock(mock: Option<&Json>) -> Result<Option<Self::Reply>, ContainerOpError> {
        mock_reply(mock, RETRIEVE_TYPE)
    }

    fn id(reply: &Self::Reply) -> &str {
        &reply.id
    }

    fn replace_id(reply: &mut Self::Reply, id: String) {
        reply.id = id;
    }
}

pub(crate) struct Delete;

impl Kind for Delete {
    type Reply = DeleteContainerResult;
    const CALL_TYPE: &'static str = DELETE_CALL_TYPE;

    fn mock(mock: Option<&Json>) -> Result<Option<Self::Reply>, ContainerOpError> {
        mock_reply(mock, DELETE_TYPE)
    }

    fn id(reply: &Self::Reply) -> &str {
        &reply.id
    }

    fn replace_id(reply: &mut Self::Reply, id: String) {
        reply.id = id;
    }
}
