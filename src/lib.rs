#![forbid(unsafe_code)]
#![no_std]
#![doc = include_str!("../README.md")]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;

use serde::{Deserialize, Deserializer, Serialize, de};
use serde_json::Value;
use thiserror::Error;

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

/// The error returned by [`parse_v1`] and [`parse_v2`].
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
    fn missing(context: &'static str, field: &'static str) -> Self {
        Self::MissingField { context, field }
    }
}

/// Rejects an array whose length does not match the one it is co-indexed with.
fn check_len(
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

/// The co-indexed column arrays of a v1 insert or update.
///
/// `columnnames` and `columnvalues` are always emitted, the rest are option-gated, and every array
/// present has the length of `columnnames`. [`parse_v1`] rejects input that does not.
///
/// No `Deserialize` impl: the fields are inlined into the change object, so it has no wire form.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ColumnArrays {
    /// Column names, in tuple order.
    pub columnnames: Vec<String>,
    /// PostgreSQL type names, absent under `include-types=false`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columntypes: Option<Vec<String>>,
    /// Type OIDs, under `include-type-oids=true`. Signed for the reason [`Column::typeoid`] is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columntypeoids: Option<Vec<i64>>,
    /// Attribute numbers, under `include-column-positions=true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columnpositions: Option<Vec<i32>>,
    /// Whether each column is nullable, under `include-not-null=true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columnoptionals: Option<Vec<bool>>,
    /// Default expressions, under `include-default=true`. An inner `None` means no `DEFAULT`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columndefaults: Option<Vec<Option<String>>>,
    /// Column values, in tuple order.
    pub columnvalues: Vec<Value>,
}

impl ColumnArrays {
    /// The two always-emitted arrays, from name and value pairs so they cannot disagree in length.
    /// Keeping a companion array co-indexed is then the caller's job.
    ///
    /// # Examples
    ///
    /// ```
    /// use wal2json_events::ColumnArrays;
    ///
    /// let mut columns = ColumnArrays::new([
    ///     ("id".to_owned(), serde_json::json!(7)),
    ///     ("email".to_owned(), serde_json::json!("a@b.c")),
    /// ]);
    /// columns.columntypes = Some(vec!["integer".to_owned(), "text".to_owned()]);
    ///
    /// assert_eq!(columns.columnnames.len(), columns.columnvalues.len());
    /// ```
    #[must_use]
    pub fn new(entries: impl IntoIterator<Item = (String, Value)>) -> Self {
        let (columnnames, columnvalues) = entries.into_iter().unzip();
        Self {
            columnnames,
            columntypes: None,
            columntypeoids: None,
            columnpositions: None,
            columnoptionals: None,
            columndefaults: None,
            columnvalues,
        }
    }

    fn check(&self) -> Result<(), ParseError> {
        let names = self.columnnames.len();
        check_len(
            "columnvalues",
            self.columnvalues.len(),
            "columnnames",
            names,
        )?;
        if let Some(types) = &self.columntypes {
            check_len("columntypes", types.len(), "columnnames", names)?;
        }
        if let Some(typeoids) = &self.columntypeoids {
            check_len("columntypeoids", typeoids.len(), "columnnames", names)?;
        }
        if let Some(positions) = &self.columnpositions {
            check_len("columnpositions", positions.len(), "columnnames", names)?;
        }
        if let Some(optionals) = &self.columnoptionals {
            check_len("columnoptionals", optionals.len(), "columnnames", names)?;
        }
        if let Some(defaults) = &self.columndefaults {
            check_len("columndefaults", defaults.len(), "columnnames", names)?;
        }
        Ok(())
    }
}

/// Old key information identifying the row in v1 update and delete changes. Co-indexed on
/// `keynames`, which [`parse_v1`] enforces.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OldKeys {
    /// Identity column names.
    pub keynames: Vec<String>,
    /// Identity PostgreSQL type names, absent under `include-types=false`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keytypes: Option<Vec<String>>,
    /// Identity type OIDs, under `include-type-oids=true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keytypeoids: Option<Vec<i64>>,
    /// Identity column values.
    pub keyvalues: Vec<Value>,
}

impl OldKeys {
    /// The two always-emitted arrays, from name and value pairs so they cannot disagree in length.
    #[must_use]
    pub fn new(entries: impl IntoIterator<Item = (String, Value)>) -> Self {
        let (keynames, keyvalues) = entries.into_iter().unzip();
        Self {
            keynames,
            keytypes: None,
            keytypeoids: None,
            keyvalues,
        }
    }
}

#[derive(Deserialize)]
struct OldKeysWire {
    keynames: Vec<String>,
    keytypes: Option<Vec<String>>,
    keytypeoids: Option<Vec<i64>>,
    keyvalues: Vec<Value>,
}

impl OldKeysWire {
    fn into_model(self) -> Result<OldKeys, ParseError> {
        let names = self.keynames.len();
        check_len("keyvalues", self.keyvalues.len(), "keynames", names)?;
        if let Some(types) = &self.keytypes {
            check_len("keytypes", types.len(), "keynames", names)?;
        }
        if let Some(typeoids) = &self.keytypeoids {
            check_len("keytypeoids", typeoids.len(), "keynames", names)?;
        }
        Ok(OldKeys {
            keynames: self.keynames,
            keytypes: self.keytypes,
            keytypeoids: self.keytypeoids,
            keyvalues: self.keyvalues,
        })
    }
}

impl<'de> Deserialize<'de> for OldKeys {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        OldKeysWire::deserialize(deserializer)?
            .into_model()
            .map_err(de::Error::custom)
    }
}

/// Primary key information of a v1 change, under `include-pk=true`.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PrimaryKeyV1 {
    /// Primary key column names.
    pub pknames: Vec<String>,
    /// Primary key type names. wal2json emits this key even under `include-types=false`, where it
    /// is empty rather than absent.
    pub pktypes: Vec<String>,
}

impl PrimaryKeyV1 {
    /// Names with no types, the shape wal2json emits under `include-types=false`. Setting
    /// `pktypes` means matching the length of `pknames`.
    #[must_use]
    pub fn new(pknames: Vec<String>) -> Self {
        Self {
            pknames,
            pktypes: Vec::new(),
        }
    }
}

#[derive(Deserialize)]
struct PrimaryKeyV1Wire {
    pknames: Vec<String>,
    pktypes: Vec<String>,
}

impl PrimaryKeyV1Wire {
    fn into_model(self) -> Result<PrimaryKeyV1, ParseError> {
        if !self.pktypes.is_empty() {
            check_len("pktypes", self.pktypes.len(), "pknames", self.pknames.len())?;
        }
        Ok(PrimaryKeyV1 {
            pknames: self.pknames,
            pktypes: self.pktypes,
        })
    }
}

impl<'de> Deserialize<'de> for PrimaryKeyV1 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        PrimaryKeyV1Wire::deserialize(deserializer)?
            .into_model()
            .map_err(de::Error::custom)
    }
}

/// A single change inside a wal2json v1 transaction.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind")]
pub enum ChangeV1 {
    /// Row insert.
    #[serde(rename = "insert")]
    Insert {
        /// Schema name, absent under `include-schemas=false`.
        #[serde(skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
        /// Table name.
        table: String,
        /// The new tuple.
        #[serde(flatten)]
        columns: ColumnArrays,
        /// Primary key information, under `include-pk=true`.
        #[serde(skip_serializing_if = "Option::is_none")]
        pk: Option<PrimaryKeyV1>,
    },
    /// Row update.
    #[serde(rename = "update")]
    Update {
        /// Schema name, absent under `include-schemas=false`.
        #[serde(skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
        /// Table name.
        table: String,
        /// The new tuple.
        #[serde(flatten)]
        columns: ColumnArrays,
        /// Primary key information, under `include-pk=true`.
        #[serde(skip_serializing_if = "Option::is_none")]
        pk: Option<PrimaryKeyV1>,
        /// The old row identity.
        oldkeys: OldKeys,
    },
    /// Row delete, which carries no column arrays.
    #[serde(rename = "delete")]
    Delete {
        /// Schema name, absent under `include-schemas=false`.
        #[serde(skip_serializing_if = "Option::is_none")]
        schema: Option<String>,
        /// Table name.
        table: String,
        /// Primary key information, under `include-pk=true`.
        #[serde(skip_serializing_if = "Option::is_none")]
        pk: Option<PrimaryKeyV1>,
        /// The old row identity.
        oldkeys: OldKeys,
    },
    /// User-defined logical message, with no schema or table.
    #[serde(rename = "message")]
    Message {
        /// Whether the message was emitted transactionally.
        transactional: bool,
        /// Message prefix, as passed to `pg_logical_emit_message`.
        prefix: String,
        /// Message content, as passed to `pg_logical_emit_message`.
        content: String,
    },
}

impl ChangeV1 {
    /// The table, for row changes.
    #[must_use]
    pub fn table(&self) -> Option<&str> {
        match self {
            Self::Insert { table, .. }
            | Self::Update { table, .. }
            | Self::Delete { table, .. } => Some(table),
            Self::Message { .. } => None,
        }
    }

    /// The schema, absent under `include-schemas=false`.
    #[must_use]
    pub fn schema(&self) -> Option<&str> {
        match self {
            Self::Insert { schema, .. }
            | Self::Update { schema, .. }
            | Self::Delete { schema, .. } => schema.as_deref(),
            Self::Message { .. } => None,
        }
    }
}

#[derive(Deserialize, Clone, Copy)]
enum KindV1 {
    #[serde(rename = "insert")]
    Insert,
    #[serde(rename = "update")]
    Update,
    #[serde(rename = "delete")]
    Delete,
    #[serde(rename = "message")]
    Message,
}

impl KindV1 {
    fn context(self) -> &'static str {
        match self {
            Self::Insert => "a v1 insert",
            Self::Update => "a v1 update",
            Self::Delete => "a v1 delete",
            Self::Message => "a v1 message",
        }
    }
}

// Flat wire form, as MessageV2Wire.
#[derive(Deserialize)]
struct ChangeV1Wire {
    kind: KindV1,
    schema: Option<String>,
    table: Option<String>,
    columnnames: Option<Vec<String>>,
    columntypes: Option<Vec<String>>,
    columntypeoids: Option<Vec<i64>>,
    columnpositions: Option<Vec<i32>>,
    columnoptionals: Option<Vec<bool>>,
    columndefaults: Option<Vec<Option<String>>>,
    columnvalues: Option<Vec<Value>>,
    pk: Option<PrimaryKeyV1Wire>,
    oldkeys: Option<OldKeysWire>,
    transactional: Option<bool>,
    prefix: Option<String>,
    content: Option<String>,
}

impl ChangeV1Wire {
    fn columns(&mut self, context: &'static str) -> Result<ColumnArrays, ParseError> {
        let columns = ColumnArrays {
            columnnames: self
                .columnnames
                .take()
                .ok_or_else(|| ParseError::missing(context, "columnnames"))?,
            columntypes: self.columntypes.take(),
            columntypeoids: self.columntypeoids.take(),
            columnpositions: self.columnpositions.take(),
            columnoptionals: self.columnoptionals.take(),
            columndefaults: self.columndefaults.take(),
            columnvalues: self
                .columnvalues
                .take()
                .ok_or_else(|| ParseError::missing(context, "columnvalues"))?,
        };
        columns.check()?;
        Ok(columns)
    }

    fn table(&mut self, context: &'static str) -> Result<String, ParseError> {
        self.table
            .take()
            .ok_or_else(|| ParseError::missing(context, "table"))
    }

    fn oldkeys(&mut self, context: &'static str) -> Result<OldKeys, ParseError> {
        self.oldkeys
            .take()
            .ok_or_else(|| ParseError::missing(context, "oldkeys"))?
            .into_model()
    }

    fn pk(&mut self) -> Result<Option<PrimaryKeyV1>, ParseError> {
        self.pk.take().map(PrimaryKeyV1Wire::into_model).transpose()
    }

    fn into_model(mut self) -> Result<ChangeV1, ParseError> {
        let context = self.kind.context();
        match self.kind {
            KindV1::Insert => Ok(ChangeV1::Insert {
                schema: self.schema.take(),
                table: self.table(context)?,
                columns: self.columns(context)?,
                pk: self.pk()?,
            }),
            KindV1::Update => Ok(ChangeV1::Update {
                schema: self.schema.take(),
                table: self.table(context)?,
                columns: self.columns(context)?,
                pk: self.pk()?,
                oldkeys: self.oldkeys(context)?,
            }),
            KindV1::Delete => Ok(ChangeV1::Delete {
                schema: self.schema.take(),
                table: self.table(context)?,
                pk: self.pk()?,
                oldkeys: self.oldkeys(context)?,
            }),
            KindV1::Message => Ok(ChangeV1::Message {
                transactional: self
                    .transactional
                    .ok_or_else(|| ParseError::missing(context, "transactional"))?,
                prefix: self
                    .prefix
                    .take()
                    .ok_or_else(|| ParseError::missing(context, "prefix"))?,
                content: self
                    .content
                    .take()
                    .ok_or_else(|| ParseError::missing(context, "content"))?,
            }),
        }
    }
}

impl<'de> Deserialize<'de> for ChangeV1 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        ChangeV1Wire::deserialize(deserializer)?
            .into_model()
            .map_err(de::Error::custom)
    }
}

/// A wal2json v1 transaction, one JSON object per transaction.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TransactionV1 {
    /// Transaction id, under `include-xids=true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xid: Option<u32>,
    /// LSN just past the transaction end, under `include-lsn=true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nextlsn: Option<String>,
    /// Commit timestamp, under `include-timestamp=true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    /// Replication origin id, under `include-origin=true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<u32>,
    /// The ordered list of changes.
    pub change: Vec<ChangeV1>,
}

impl TransactionV1 {
    /// A transaction holding the given changes.
    #[must_use]
    pub fn new(change: Vec<ChangeV1>) -> Self {
        Self {
            xid: None,
            nextlsn: None,
            timestamp: None,
            origin: None,
            change,
        }
    }
}

// Changes stay in wire form so `parse_v1` reports a typed error rather than a serde message.
#[derive(Deserialize)]
struct TransactionV1Wire {
    xid: Option<u32>,
    nextlsn: Option<String>,
    timestamp: Option<String>,
    origin: Option<u32>,
    change: Vec<ChangeV1Wire>,
}

impl TransactionV1Wire {
    fn into_model(self) -> Result<TransactionV1, ParseError> {
        Ok(TransactionV1 {
            xid: self.xid,
            nextlsn: self.nextlsn,
            timestamp: self.timestamp,
            origin: self.origin,
            change: self
                .change
                .into_iter()
                .map(ChangeV1Wire::into_model)
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}

impl<'de> Deserialize<'de> for TransactionV1 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        TransactionV1Wire::deserialize(deserializer)?
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

/// Parse a wal2json v1 transaction from JSON.
///
/// # Errors
///
/// [`ParseError::Json`] on malformed JSON, [`ParseError::MissingField`] on a change missing a field
/// its kind requires, [`ParseError::LengthMismatch`] on arrays of unequal length.
///
/// # Examples
///
/// ```
/// use wal2json_events::{ChangeV1, parse_v1};
///
/// let json = r#"{"change":[{"kind":"delete","schema":"public","table":"users",
///     "oldkeys":{"keynames":["id"],"keytypes":["integer"],"keyvalues":[7]}}]}"#;
///
/// let ChangeV1::Delete { oldkeys, .. } = &parse_v1(json)?.change[0] else {
///     panic!("expected a delete");
/// };
/// assert_eq!(oldkeys.keynames, ["id"]);
/// # Ok::<(), wal2json_events::ParseError>(())
/// ```
pub fn parse_v1(json: &str) -> Result<TransactionV1, ParseError> {
    serde_json::from_str::<TransactionV1Wire>(json)?.into_model()
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

/// Parse a wal2json v1 transaction from JSON held as bytes.
///
/// # Errors
///
/// As [`parse_v1`].
///
/// # Examples
///
/// ```
/// use wal2json_events::parse_v1_slice;
///
/// let json: &[u8] = br#"{"xid":749,"change":[]}"#;
///
/// assert_eq!(parse_v1_slice(json)?.xid, Some(749));
/// # Ok::<(), wal2json_events::ParseError>(())
/// ```
pub fn parse_v1_slice(json: &[u8]) -> Result<TransactionV1, ParseError> {
    serde_json::from_slice::<TransactionV1Wire>(json)?.into_model()
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

/// Parse every transaction of a wal2json v1 stream, which carries one transaction per line.
///
/// As [`parse_v2_lines`], over whole transactions.
///
/// # Examples
///
/// ```
/// use wal2json_events::parse_v1_lines;
///
/// let stream = "{\"xid\":1,\"change\":[]}\n{\"xid\":2,\"change\":[]}\n";
/// let ids: Vec<_> = parse_v1_lines(stream)
///     .map(|transaction| transaction.map(|transaction| transaction.xid))
///     .collect::<Result<_, _>>()?;
///
/// assert_eq!(ids, [Some(1), Some(2)]);
/// # Ok::<(), wal2json_events::ParseError>(())
/// ```
pub fn parse_v1_lines(stream: &str) -> impl Iterator<Item = Result<TransactionV1, ParseError>> {
    stream
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(parse_v1)
}
