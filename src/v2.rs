//! Format version 2: one JSON object per message, discriminated by an action letter.

use alloc::string::String;
use alloc::vec::Vec;

use serde::{Deserialize, Deserializer, Serialize, de};
use serde_json::Value;

use crate::error::ParseError;

/// wal2json v2 action, one variant per wire letter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    /// Begin transaction, wire letter `B`.
    #[serde(rename = "B")]
    Begin,
    /// Commit transaction, wire letter `C`.
    #[serde(rename = "C")]
    Commit,
    /// Row insert, wire letter `I`.
    #[serde(rename = "I")]
    Insert,
    /// Row update, wire letter `U`.
    #[serde(rename = "U")]
    Update,
    /// Row delete, wire letter `D`.
    #[serde(rename = "D")]
    Delete,
    /// Table truncation, wire letter `T`.
    #[serde(rename = "T")]
    Truncate,
    /// User-defined logical message, wire letter `M`.
    #[serde(rename = "M")]
    Message,
}

/// Deserializes a present field into `Some`, so an explicit `null` stays distinct from an absent
/// key: `"value":null` is a SQL NULL, no key means the option emitting the field was off.
fn present<'de, T, D>(deserializer: D) -> Result<Option<T>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    T::deserialize(deserializer).map(Some)
}

/// wal2json v2 column. Everything but `name` is option-gated or action-specific, hence optional.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Column {
    /// Column name.
    pub name: String,
    /// PostgreSQL type name, absent under `include-types=false`.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
    /// Type OID, under `include-type-oids=true`. Signed: wal2json prints it with `%d`, so an OID
    /// above `i32::MAX` arrives negative.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub typeoid: Option<i64>,
    /// Column value, absent for `pk` entries. `Some(Value::Null)` is a SQL NULL.
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub value: Option<Value>,
    /// Whether the column is nullable, under `include-not-null=true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub optional: Option<bool>,
    /// Attribute number, under `include-column-positions=true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<i32>,
    /// Default expression, under `include-default=true`. Inner `None` means no `DEFAULT` clause.
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub default: Option<Option<String>>,
}

impl Column {
    /// A column with only its name set. Every other field is public and starts absent.
    ///
    /// # Examples
    ///
    /// ```
    /// use wal2json_events::Column;
    ///
    /// let mut column = Column::new("id");
    /// column.type_name = Some("integer".to_owned());
    /// column.value = Some(serde_json::json!(7));
    ///
    /// assert_eq!(column.name, "id");
    /// ```
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            type_name: None,
            typeoid: None,
            value: None,
            optional: None,
            position: None,
            default: None,
        }
    }
}

/// Transaction boundary payload, carried by the v2 `B` and `C` actions.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TransactionBoundary {
    /// Transaction id, under `include-xids=true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xid: Option<u32>,
    /// Commit timestamp, under `include-timestamp=true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    /// Replication origin id, under `include-origin=true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<u32>,
    /// Record LSN, under `include-lsn=true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lsn: Option<String>,
    /// LSN just past the transaction end, under `include-lsn=true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nextlsn: Option<String>,
}

impl TransactionBoundary {
    /// A boundary with every field absent.
    #[must_use]
    pub fn new() -> Self {
        Self {
            xid: None,
            timestamp: None,
            origin: None,
            lsn: None,
            nextlsn: None,
        }
    }
}

impl Default for TransactionBoundary {
    fn default() -> Self {
        Self::new()
    }
}

/// wal2json v2 row change, carried by the `I`, `U` and `D` actions.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RowV2 {
    /// Transaction id, under `include-xids=true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xid: Option<u32>,
    /// Commit timestamp, under `include-timestamp=true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    /// Replication origin id, under `include-origin=true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<u32>,
    /// Record LSN, under `include-lsn=true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lsn: Option<String>,
    /// Schema name, absent under `include-schemas=false`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    /// Table name.
    pub table: String,
    /// New tuple, for insert and update. Unchanged out-of-line values are omitted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columns: Option<Vec<Column>>,
    /// Old row identity, for update and delete.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity: Option<Vec<Column>>,
    /// Primary key columns, under `include-pk=true`. These carry no `value`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pk: Option<Vec<Column>>,
}

impl RowV2 {
    /// A row with only its table set.
    #[must_use]
    pub fn new(table: impl Into<String>) -> Self {
        Self {
            xid: None,
            timestamp: None,
            origin: None,
            lsn: None,
            schema: None,
            table: table.into(),
            columns: None,
            identity: None,
            pk: None,
        }
    }
}

/// wal2json v2 truncation, one message per truncated table.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TruncateV2 {
    /// Transaction id, under `include-xids=true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xid: Option<u32>,
    /// Commit timestamp, under `include-timestamp=true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    /// Replication origin id, under `include-origin=true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<u32>,
    /// Record LSN, under `include-lsn=true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lsn: Option<String>,
    /// Schema name, absent under `include-schemas=false`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    /// Table name.
    pub table: String,
}

impl TruncateV2 {
    /// A truncation with only its table set.
    #[must_use]
    pub fn new(table: impl Into<String>) -> Self {
        Self {
            xid: None,
            timestamp: None,
            origin: None,
            lsn: None,
            schema: None,
            table: table.into(),
        }
    }
}

/// wal2json v2 logical message, carried by the `M` action.
///
/// For a non-transactional message wal2json writes `xid`, `timestamp` and `origin` as explicit
/// nulls. They parse to `None` and serialize back as absent, which `transactional: false` implies.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LogicalMessageV2 {
    /// Transaction id, under `include-xids=true` and only when transactional.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xid: Option<u32>,
    /// Commit timestamp, under `include-timestamp=true` and only when transactional.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    /// Replication origin id, under `include-origin=true` and only when transactional.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<u32>,
    /// Record LSN, under `include-lsn=true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lsn: Option<String>,
    /// Whether the message was emitted transactionally.
    pub transactional: bool,
    /// Message prefix, as passed to `pg_logical_emit_message`.
    pub prefix: String,
    /// Message content, as passed to `pg_logical_emit_message`.
    pub content: String,
}

impl LogicalMessageV2 {
    /// A message with the three fields wal2json always emits.
    #[must_use]
    pub fn new(transactional: bool, prefix: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            xid: None,
            timestamp: None,
            origin: None,
            lsn: None,
            transactional,
            prefix: prefix.into(),
            content: content.into(),
        }
    }
}

/// A single wal2json v2 message, one JSON object per line.
///
/// Exhaustive, so a `match` needs no catch-all and a future action letter becomes a compile error.
///
/// # Examples
///
/// ```
/// use wal2json_events::{MessageV2, parse_v2};
///
/// let message = parse_v2(r#"{"action":"C","xid":749}"#)?;
///
/// // Every row action carries the same payload, so one arm can cover all three.
/// let subject = match message {
///     MessageV2::Begin(_) => "transaction start".to_owned(),
///     MessageV2::Commit(_) => "transaction commit".to_owned(),
///     MessageV2::Insert(row) | MessageV2::Update(row) | MessageV2::Delete(row) => row.table,
///     MessageV2::Truncate(truncate) => truncate.table,
///     MessageV2::Message(logical) => logical.prefix,
/// };
///
/// assert_eq!(subject, "transaction commit");
/// # Ok::<(), wal2json_events::ParseError>(())
/// ```
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "action")]
pub enum MessageV2 {
    /// Transaction start, emitted under `include-transaction=true`.
    #[serde(rename = "B")]
    Begin(TransactionBoundary),
    /// Transaction end, emitted under `include-transaction=true`.
    #[serde(rename = "C")]
    Commit(TransactionBoundary),
    /// Row insert.
    #[serde(rename = "I")]
    Insert(RowV2),
    /// Row update.
    #[serde(rename = "U")]
    Update(RowV2),
    /// Row delete.
    #[serde(rename = "D")]
    Delete(RowV2),
    /// Table truncation.
    #[serde(rename = "T")]
    Truncate(TruncateV2),
    /// User-defined logical message.
    #[serde(rename = "M")]
    Message(LogicalMessageV2),
}

impl MessageV2 {
    /// The wire action of this message.
    #[must_use]
    pub fn action(&self) -> Action {
        match self {
            Self::Begin(_) => Action::Begin,
            Self::Commit(_) => Action::Commit,
            Self::Insert(_) => Action::Insert,
            Self::Update(_) => Action::Update,
            Self::Delete(_) => Action::Delete,
            Self::Truncate(_) => Action::Truncate,
            Self::Message(_) => Action::Message,
        }
    }

    /// The table, for row and truncate actions.
    #[must_use]
    pub fn table(&self) -> Option<&str> {
        match self {
            Self::Insert(row) | Self::Update(row) | Self::Delete(row) => Some(&row.table),
            Self::Truncate(truncate) => Some(&truncate.table),
            Self::Begin(_) | Self::Commit(_) | Self::Message(_) => None,
        }
    }

    /// The schema, absent under `include-schemas=false`.
    #[must_use]
    pub fn schema(&self) -> Option<&str> {
        match self {
            Self::Insert(row) | Self::Update(row) | Self::Delete(row) => row.schema.as_deref(),
            Self::Truncate(truncate) => truncate.schema.as_deref(),
            Self::Begin(_) | Self::Commit(_) | Self::Message(_) => None,
        }
    }
}

// Flat wire form: every emittable field once, with the action deciding which are required.
#[derive(Deserialize)]
struct MessageV2Wire {
    action: Action,
    xid: Option<u32>,
    timestamp: Option<String>,
    origin: Option<u32>,
    lsn: Option<String>,
    nextlsn: Option<String>,
    schema: Option<String>,
    table: Option<String>,
    columns: Option<Vec<Column>>,
    identity: Option<Vec<Column>>,
    pk: Option<Vec<Column>>,
    transactional: Option<bool>,
    prefix: Option<String>,
    content: Option<String>,
}

impl MessageV2Wire {
    fn into_model(self) -> Result<MessageV2, ParseError> {
        let Self {
            action,
            xid,
            timestamp,
            origin,
            lsn,
            nextlsn,
            schema,
            table,
            columns,
            identity,
            pk,
            transactional,
            prefix,
            content,
        } = self;

        match action {
            Action::Begin | Action::Commit => {
                let boundary = TransactionBoundary {
                    xid,
                    timestamp,
                    origin,
                    lsn,
                    nextlsn,
                };
                Ok(match action {
                    Action::Begin => MessageV2::Begin(boundary),
                    _ => MessageV2::Commit(boundary),
                })
            }
            Action::Insert | Action::Update | Action::Delete => {
                let row = RowV2 {
                    xid,
                    timestamp,
                    origin,
                    lsn,
                    schema,
                    table: table.ok_or_else(|| ParseError::missing("a v2 row action", "table"))?,
                    columns,
                    identity,
                    pk,
                };
                Ok(match action {
                    Action::Insert => MessageV2::Insert(row),
                    Action::Update => MessageV2::Update(row),
                    _ => MessageV2::Delete(row),
                })
            }
            Action::Truncate => Ok(MessageV2::Truncate(TruncateV2 {
                xid,
                timestamp,
                origin,
                lsn,
                schema,
                table: table.ok_or_else(|| ParseError::missing("a v2 truncate action", "table"))?,
            })),
            Action::Message => {
                const CONTEXT: &str = "a v2 message action";
                Ok(MessageV2::Message(LogicalMessageV2 {
                    xid,
                    timestamp,
                    origin,
                    lsn,
                    transactional: transactional
                        .ok_or_else(|| ParseError::missing(CONTEXT, "transactional"))?,
                    prefix: prefix.ok_or_else(|| ParseError::missing(CONTEXT, "prefix"))?,
                    content: content.ok_or_else(|| ParseError::missing(CONTEXT, "content"))?,
                }))
            }
        }
    }
}

impl<'de> Deserialize<'de> for MessageV2 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        MessageV2Wire::deserialize(deserializer)?
            .into_model()
            .map_err(de::Error::custom)
    }
}

/// Parse a wal2json v2 message from a JSON line.
///
/// # Errors
///
/// [`ParseError::Json`] on malformed JSON, [`ParseError::MissingField`] on a row or truncate
/// without a `table`, or a message without `transactional`, `prefix` or `content`.
///
/// # Examples
///
/// ```
/// use wal2json_events::{MessageV2, parse_v2};
///
/// let line = r#"{"action":"D","schema":"public","table":"users","identity":[{"name":"id","type":"integer","value":1}]}"#;
///
/// let MessageV2::Delete(row) = parse_v2(line)? else {
///     panic!("expected a delete");
/// };
/// assert_eq!(row.table, "users");
/// assert_eq!(row.identity.unwrap()[0].name, "id");
/// # Ok::<(), wal2json_events::ParseError>(())
/// ```
pub fn parse_v2(line: &str) -> Result<MessageV2, ParseError> {
    serde_json::from_str::<MessageV2Wire>(line)?.into_model()
}

/// Parse a wal2json v2 message from a line held as bytes, as it arrives from a connection.
///
/// # Errors
///
/// As [`parse_v2`].
///
/// # Examples
///
/// ```
/// use wal2json_events::parse_v2_slice;
///
/// let line: &[u8] = br#"{"action":"T","schema":"public","table":"users"}"#;
///
/// assert_eq!(parse_v2_slice(line)?.table(), Some("users"));
/// # Ok::<(), wal2json_events::ParseError>(())
/// ```
pub fn parse_v2_slice(line: &[u8]) -> Result<MessageV2, ParseError> {
    serde_json::from_slice::<MessageV2Wire>(line)?.into_model()
}

/// Parse every message of a wal2json v2 stream, which carries one message per line.
///
/// Blank lines are skipped and each message is a separate result, so one unparseable line does not
/// end the iteration. Assumes wal2json's default output: `pretty-print` spans lines.
///
/// # Examples
///
/// ```
/// use wal2json_events::parse_v2_lines;
///
/// let stream = "{\"action\":\"B\"}\n{\"action\":\"nonsense\"}\n{\"action\":\"C\"}\n";
/// let results: Vec<_> = parse_v2_lines(stream).collect();
///
/// // The bad line is one error among three results, not the end of the stream.
/// assert_eq!(results.len(), 3);
/// assert!(results[0].is_ok() && results[1].is_err() && results[2].is_ok());
/// ```
pub fn parse_v2_lines(stream: &str) -> impl Iterator<Item = Result<MessageV2, ParseError>> {
    stream
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(parse_v2)
}
