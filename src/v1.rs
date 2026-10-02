//! Format version 1: one JSON object per transaction, with rows as parallel arrays.

use alloc::string::String;
use alloc::vec::Vec;

use serde::{Deserialize, Deserializer, Serialize, de};
use serde_json::Value;

use crate::error::{ParseError, check_len};

/// The co-indexed column arrays of a v1 insert or update.
///
/// `columnnames` and `columnvalues` are always emitted, the rest are option-gated, and every array
/// present has the length of `columnnames`. [`parse_v1`] rejects input that does not.
///
/// No `Deserialize` impl: the fields are inlined into the change object, so it has no wire form.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct ColumnArrays {
    /// Column names, in tuple order.
    pub columnnames: Vec<String>,
    /// PostgreSQL type names, absent under `include-types=false`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columntypes: Option<Vec<String>>,
    /// Type OIDs, under `include-type-oids=true`. Signed for the reason [`crate::Column::typeoid`] is.
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
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
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

/// Parse every transaction of a wal2json v1 stream, which carries one transaction per line.
///
/// As [`crate::parse_v2_lines`], over whole transactions.
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
