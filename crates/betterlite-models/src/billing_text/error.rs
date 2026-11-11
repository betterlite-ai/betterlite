use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum BillingTextError {
    #[error("the {field} field is required")]
    Missing { field: &'static str },
    #[error("the {field} field must be a string")]
    NotText { field: &'static str },
    #[error("the {field} field must be an object")]
    NotObject { field: &'static str },
    #[error("the {field} field must be a list")]
    NotList { field: &'static str },
    #[error("the {field} field must be a string or a list of strings")]
    NotTextOrList { field: &'static str },
}
