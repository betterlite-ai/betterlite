use super::block_on;
use super::fixture::leaf_error;
use crate::azure_resource_dispatch::{AzureResourceError, AzureResourceOperation, dispatch};

#[test]
fn azure_resource_dispatch_unsupported_operation() {
    let error = leaf_error(block_on(dispatch(AzureResourceOperation::Unsupported {
        operation: "vector_stores.unknown",
    })));
    assert_eq!(
        error.to_string(),
        "betterlite does not run the vector_stores.unknown operation through the azure resource provider"
    );
    assert!(matches!(
        error,
        AzureResourceError::Unsupported {
            operation: "vector_stores.unknown"
        }
    ));
}
