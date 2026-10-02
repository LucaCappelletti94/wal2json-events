//! The error both parsers return.

use thiserror::Error;

/// The error returned by [`crate::parse_v1`] and [`crate::parse_v2`].
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum ParseError {
    /// The input is not valid JSON, or a value has the wrong JSON type.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// A field that this action or kind requires is absent.
    #[error("{context} requires the field `{field}`")]
    MissingField {
        /// The action or kind that requires the field.
        context: &'static str,
        /// The wire name of the absent field.
        field: &'static str,
    },
    /// Two co-indexed v1 arrays disagree in length.
    #[error("`{field}` has length {len} but `{reference}` has length {expected}")]
    LengthMismatch {
        /// The wire name of the array that disagrees.
        field: &'static str,
        /// Its length.
        len: usize,
        /// The wire name of the array it must be co-indexed with.
        reference: &'static str,
        /// That array's length.
        expected: usize,
    },
}

impl ParseError {
    pub(crate) fn missing(context: &'static str, field: &'static str) -> Self {
        Self::MissingField { context, field }
    }
}

/// Rejects an array whose length does not match the one it is co-indexed with.
pub(crate) fn check_len(
    field: &'static str,
    len: usize,
    reference: &'static str,
    expected: usize,
) -> Result<(), ParseError> {
    if len == expected {
        Ok(())
    } else {
        Err(ParseError::LengthMismatch {
            field,
            len,
            reference,
            expected,
        })
    }
}
