//! Errors raised by the sans-IO core.

use thiserror::Error;

/// Errors raised by row operations, decoding, and the effect protocol.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Error {
    /// The process is not in the queue, or the queue does not exist.
    #[error("ticket not found for process_id {0}")]
    NotFound(String),

    /// The stored item belongs to a different structure type.
    #[error("unrecognised structure type: expected {expected}, found {found}")]
    WrongType {
        /// The type the caller asked for.
        expected: &'static str,
        /// The `Type` attribute found on the item.
        found: String,
    },

    /// A required attribute is absent from the stored item.
    #[error("missing attribute {0}")]
    MissingAttribute(&'static str),

    /// An attribute is present but has the wrong type or an unparseable value.
    #[error("invalid attribute {name}: {reason}")]
    InvalidAttribute {
        /// The attribute name.
        name: &'static str,
        /// Why the value was rejected.
        reason: String,
    },

    /// A machine received an input that does not answer its pending effect,
    /// or was stepped after it finished.
    #[error("protocol error: {0}")]
    Protocol(&'static str),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_found_display_text_is_stable() {
        assert_eq!(
            Error::NotFound("nobody".into()).to_string(),
            "ticket not found for process_id nobody"
        );
    }

    #[test]
    fn wrong_type_names_both_types() {
        let e = Error::WrongType {
            expected: "COUNTER",
            found: "QUEUE".into(),
        };
        assert_eq!(
            e.to_string(),
            "unrecognised structure type: expected COUNTER, found QUEUE"
        );
    }

    #[test]
    fn attribute_errors_name_the_attribute() {
        assert_eq!(
            Error::MissingAttribute("Version").to_string(),
            "missing attribute Version"
        );
        let e = Error::InvalidAttribute {
            name: "Value",
            reason: "not an unsigned integer: -1".into(),
        };
        assert_eq!(
            e.to_string(),
            "invalid attribute Value: not an unsigned integer: -1"
        );
    }
}
