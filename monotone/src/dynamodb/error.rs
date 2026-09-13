use aws_sdk_dynamodb::error::DisplayErrorContext;
use thiserror::Error;

/// Errors from the DynamoDB backend.
#[derive(Debug, Error)]
pub enum Error {
    /// A row, decoding, or protocol error from the shared core.
    #[error(transparent)]
    Core(#[from] crate::core::error::Error),

    /// `DescribeTable` reported that the table does not exist.
    #[error("table not found: {0}")]
    TableNotFound(String),

    /// `CreateTable` reported that the table already exists.
    #[error("table already exists: {0}")]
    TableAlreadyExists(String),

    /// A conditional `PutItem` was rejected because the version had changed.
    #[error("conditional update failed")]
    ConditionalUpdateFailed,

    /// A table call succeeded but returned no table description.
    #[error("no table description returned for {0}")]
    NoTableInfo(String),

    /// A request could not be built.
    #[error("invalid DynamoDB request: {0}")]
    Build(#[from] aws_sdk_dynamodb::error::BuildError),

    /// Any other service, transport, or credential failure.
    #[error("DynamoDB {operation} failed: {message}")]
    Sdk {
        /// The DynamoDB operation that failed.
        operation: &'static str,
        /// The full error chain, rendered for display.
        message: String,
        /// The original SDK error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },
}

impl Error {
    pub(crate) fn sdk<E>(operation: &'static str, error: E) -> Error
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Error::Sdk {
            operation,
            message: DisplayErrorContext(&error).to_string(),
            source: Box::new(error),
        }
    }
}
