#![forbid(unsafe_code)]
#![no_std]
#![doc = include_str!("../README.md")]

extern crate alloc;

mod error;
mod v1;
mod v2;

pub use error::ParseError;
pub use v1::{
    ChangeV1, ColumnArrays, OldKeys, PrimaryKeyV1, TransactionV1, parse_v1, parse_v1_lines,
    parse_v1_slice,
};
pub use v2::{
    Action, Column, LogicalMessageV2, MessageV2, RowV2, TransactionBoundary, TruncateV2, parse_v2,
    parse_v2_lines, parse_v2_slice,
};
