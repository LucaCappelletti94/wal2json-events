#![allow(missing_docs)]

use serde_json::{Value, json};
use wal2json_events::{
    Action, ChangeV1, Column, ColumnArrays, MessageV2, OldKeys, RowV2, TransactionV1, TruncateV2,
    parse_v1, parse_v2,
};

const V1_FIXTURE: &str = include_str!("fixtures/wal2json-v1.json");
const V2_FIXTURE: &str = include_str!("fixtures/wal2json-v2.jsonl");
const V1_ALL_OPTIONS: &str = include_str!("fixtures/wal2json-v1-all-options.jsonl");
const V2_ALL_OPTIONS: &str = include_str!("fixtures/wal2json-v2-all-options.jsonl");

fn column(name: &str, type_name: &str, value: Value) -> Column {
    let mut column = Column::new(name);
    column.type_name = Some(type_name.to_owned());
    column.value = Some(value);
    column
}

fn default_columns(status: &str, include_toast: bool) -> Vec<Column> {
    let mut columns = vec![
        column("tenant_id", "integer", json!(7)),
        column("id", "integer", json!(42)),
        column("payload", "bytea", json!("deadbeef")),
        column(
            "amount",
            "numeric",
            serde_json::from_str("12345678901234567890.123456789").unwrap(),
        ),
        column(
            "observed_at",
            "timestamp with time zone",
            json!("2026-08-28 12:34:56+05:30"),
        ),
        column("nullable_text", "text", Value::Null),
    ];
    if include_toast {
        columns.push(column("toasted_text", "text", json!("x".repeat(10_000))));
    }
    columns.push(column("status", "text", json!(status)));
    columns
}

fn full_columns(payload: &str) -> Vec<Column> {
    vec![
        column("id", "integer", json!(9)),
        column("code", "text", json!("full")),
        column("payload", "text", json!(payload)),
    ]
}

fn key_columns() -> Vec<Column> {
    vec![
        column("tenant_id", "integer", json!(7)),
        column("id", "integer", json!(42)),
    ]
}

fn arrays(columns: &[Column]) -> ColumnArrays {
    let mut arrays = ColumnArrays::new(
        columns
            .iter()
            .map(|column| (column.name.clone(), column.value.clone().unwrap())),
    );
    arrays.columntypes = Some(
        columns
            .iter()
            .map(|column| column.type_name.clone().unwrap())
            .collect(),
    );
    arrays
}

fn old_keys(columns: &[Column]) -> OldKeys {
    let mut keys = OldKeys::new(
        columns
            .iter()
            .map(|column| (column.name.clone(), column.value.clone().unwrap())),
    );
    keys.keytypes = Some(
        columns
            .iter()
            .map(|column| column.type_name.clone().unwrap())
            .collect(),
    );
    keys
}

fn insert_v1(table: &str, columns: &[Column]) -> ChangeV1 {
    ChangeV1::Insert {
        schema: Some("public".to_owned()),
        table: table.to_owned(),
        columns: arrays(columns),
        pk: None,
    }
}

fn update_v1(table: &str, columns: &[Column], keys: &[Column]) -> ChangeV1 {
    ChangeV1::Update {
        schema: Some("public".to_owned()),
        table: table.to_owned(),
        columns: arrays(columns),
        pk: None,
        oldkeys: old_keys(keys),
    }
}

fn delete_v1(table: &str, keys: &[Column]) -> ChangeV1 {
    ChangeV1::Delete {
        schema: Some("public".to_owned()),
        table: table.to_owned(),
        pk: None,
        oldkeys: old_keys(keys),
    }
}

fn expected_v1() -> TransactionV1 {
    let keys = key_columns();
    let full_before = full_columns("before");
    let full_after = full_columns("after");

    TransactionV1::new(vec![
        insert_v1("default_rows", &default_columns("inserted", true)),
        update_v1("default_rows", &default_columns("updated", false), &keys),
        delete_v1("default_rows", &keys),
        insert_v1("full_rows", &full_before),
        update_v1("full_rows", &full_after, &full_before),
        delete_v1("full_rows", &full_after),
        insert_v1("truncate_rows", &[column("id", "integer", json!(1))]),
    ])
}

fn row(table: &str, columns: Option<Vec<Column>>, identity: Option<Vec<Column>>) -> RowV2 {
    let mut row = RowV2::new(table);
    row.schema = Some("public".to_owned());
    row.columns = columns;
    row.identity = identity;
    row
}

fn expected_v2() -> Vec<MessageV2> {
    let full_before = full_columns("before");
    let full_after = full_columns("after");

    vec![
        MessageV2::Insert(row(
            "default_rows",
            Some(default_columns("inserted", true)),
            None,
        )),
        MessageV2::Update(row(
            "default_rows",
            Some(default_columns("updated", false)),
            Some(key_columns()),
        )),
        MessageV2::Delete(row("default_rows", None, Some(key_columns()))),
        MessageV2::Insert(row("full_rows", Some(full_before.clone()), None)),
        MessageV2::Update(row(
            "full_rows",
            Some(full_after.clone()),
            Some(full_before),
        )),
        MessageV2::Delete(row("full_rows", None, Some(full_after))),
        MessageV2::Insert(row(
            "truncate_rows",
            Some(vec![column("id", "integer", json!(1))]),
            None,
        )),
        MessageV2::Truncate({
            let mut truncate = TruncateV2::new("truncate_rows");
            truncate.schema = Some("public".to_owned());
            truncate
        }),
    ]
}

#[test]
fn captured_v1_parses_to_complete_structure() {
    assert_eq!(parse_v1(V1_FIXTURE).unwrap(), expected_v1());
}

#[test]
fn captured_v2_parses_to_complete_structures() {
    let actual = V2_FIXTURE
        .lines()
        .map(|line| parse_v2(line).unwrap())
        .collect::<Vec<_>>();

    assert_eq!(actual, expected_v2());
}

/// The captured JSON, minus the identifiers wal2json writes as explicit nulls on a non-transactional
/// message, which parse and serialize as absent. A null `value` or `default` is data and stays.
fn expected_wire(line: &str) -> Value {
    let mut value: Value = serde_json::from_str(line).unwrap();
    if let Value::Object(map) = &mut value {
        map.retain(|key, field| {
            !(field.is_null() && matches!(key.as_str(), "xid" | "timestamp" | "origin"))
        });
    }
    value
}

/// Nothing dropped on the way in, no null invented on the way out.
#[test]
fn captured_v1_reserializes_to_the_captured_json() {
    let parsed = parse_v1(V1_FIXTURE).unwrap();

    assert_eq!(
        serde_json::to_value(parsed).unwrap(),
        expected_wire(V1_FIXTURE)
    );
}

#[test]
fn captured_v2_reserializes_to_the_captured_json() {
    for line in V2_FIXTURE.lines() {
        let parsed = parse_v2(line).unwrap();

        assert_eq!(
            serde_json::to_value(parsed).unwrap(),
            expected_wire(line),
            "reserialization differs for line: {line}"
        );
    }
}

// Captured with every field-adding option, which is what makes the coverage claim testable.

#[test]
fn all_options_v1_reserializes_to_the_captured_json() {
    for line in V1_ALL_OPTIONS.lines() {
        let parsed = parse_v1(line).unwrap();

        assert_eq!(
            serde_json::to_value(parsed).unwrap(),
            expected_wire(line),
            "reserialization differs for line: {line}"
        );
    }
}

#[test]
fn all_options_v2_reserializes_to_the_captured_json() {
    for line in V2_ALL_OPTIONS.lines() {
        let parsed = parse_v2(line).unwrap();

        assert_eq!(
            serde_json::to_value(parsed).unwrap(),
            expected_wire(line),
            "reserialization differs for line: {line}"
        );
    }
}

// These guard the corpus itself: a regeneration that lost an option would still reserialize.

fn all_options_v2() -> Vec<MessageV2> {
    V2_ALL_OPTIONS
        .lines()
        .map(|line| parse_v2(line).unwrap())
        .collect()
}

#[test]
fn all_options_v2_corpus_covers_every_action() {
    let actions: Vec<Action> = all_options_v2().iter().map(MessageV2::action).collect();

    for action in [
        Action::Begin,
        Action::Commit,
        Action::Insert,
        Action::Update,
        Action::Delete,
        Action::Truncate,
        Action::Message,
    ] {
        assert!(
            actions.contains(&action),
            "corpus has no {action:?} message"
        );
    }
}

#[test]
fn all_options_v2_corpus_covers_the_transaction_fields() {
    let messages = all_options_v2();
    let boundary = messages
        .iter()
        .find_map(|message| match message {
            MessageV2::Begin(boundary) => Some(boundary),
            _ => None,
        })
        .expect("a begin");

    assert!(boundary.xid.is_some(), "include-xids");
    assert!(boundary.timestamp.is_some(), "include-timestamp");
    assert!(boundary.origin.is_some(), "include-origin");
    assert!(
        boundary.lsn.is_some() && boundary.nextlsn.is_some(),
        "include-lsn"
    );

    let row = messages
        .iter()
        .find_map(|message| match message {
            MessageV2::Insert(row) => Some(row),
            _ => None,
        })
        .expect("an insert");
    assert!(row.xid.is_some() && row.lsn.is_some(), "row identifiers");
    assert!(row.pk.is_some(), "include-pk");

    let update = messages
        .iter()
        .find_map(|message| match message {
            MessageV2::Update(row) => Some(row),
            _ => None,
        })
        .expect("an update");
    assert!(update.identity.is_some(), "update identity");
}

#[test]
fn all_options_v2_corpus_covers_the_column_fields() {
    let messages = all_options_v2();
    let row = messages
        .iter()
        .find_map(|message| match message {
            MessageV2::Insert(row) => Some(row),
            _ => None,
        })
        .expect("an insert");
    let columns = row.columns.as_deref().expect("insert columns");

    assert!(
        columns.iter().all(|c| c.type_name.is_some()),
        "include-types"
    );
    assert!(
        columns.iter().all(|c| c.typeoid.is_some()),
        "include-type-oids"
    );
    assert!(
        columns.iter().all(|c| c.position.is_some()),
        "include-column-positions"
    );
    assert!(
        columns.iter().all(|c| c.optional.is_some()),
        "include-not-null"
    );
    assert!(
        columns.iter().any(|c| c.default == Some(None)),
        "a column with no DEFAULT clause"
    );
    assert!(
        columns
            .iter()
            .any(|c| matches!(&c.default, Some(Some(expression)) if expression.contains("unset"))),
        "a column with a DEFAULT clause"
    );
    assert!(
        columns.iter().any(|c| c.value == Some(Value::Null)),
        "a SQL NULL value"
    );
    assert!(
        row.pk
            .as_deref()
            .expect("pk")
            .iter()
            .all(|c| c.value.is_none()),
        "pk entries carry no value"
    );
}

#[test]
fn all_options_v2_corpus_covers_both_message_forms() {
    let messages = all_options_v2();
    let logical: Vec<_> = messages
        .iter()
        .filter_map(|message| match message {
            MessageV2::Message(logical) => Some(logical),
            _ => None,
        })
        .collect();

    assert_eq!(
        logical.len(),
        2,
        "a transactional and a non-transactional message"
    );
    assert!(
        logical.iter().any(|m| m.transactional),
        "transactional message"
    );
    assert!(
        logical
            .iter()
            .any(|m| !m.transactional && m.xid.is_none() && m.lsn.is_some()),
        "non-transactional message, whose identifiers arrive as nulls"
    );
}

#[test]
fn all_options_v1_corpus_exercises_every_kind_and_array() {
    let transactions: Vec<_> = V1_ALL_OPTIONS
        .lines()
        .map(|l| parse_v1(l).unwrap())
        .collect();

    // A non-transactional message is a transaction carrying only that message.
    assert!(
        transactions.iter().any(|tx| {
            tx.xid.is_none() && matches!(tx.change.as_slice(), [ChangeV1::Message { .. }])
        }),
        "corpus has no standalone message transaction"
    );

    let tx = transactions
        .iter()
        .find(|tx| tx.change.len() > 1)
        .expect("the DML transaction");
    assert!(tx.xid.is_some(), "include-xids");
    assert!(tx.timestamp.is_some(), "include-timestamp");
    assert!(tx.origin.is_some(), "include-origin");
    assert!(tx.nextlsn.is_some(), "include-lsn");

    let (columns, pk) = tx
        .change
        .iter()
        .find_map(|change| match change {
            ChangeV1::Insert { columns, pk, .. } => Some((columns, pk)),
            _ => None,
        })
        .expect("an insert");
    assert!(columns.columntypes.is_some(), "include-types");
    assert!(columns.columntypeoids.is_some(), "include-type-oids");
    assert!(
        columns.columnpositions.is_some(),
        "include-column-positions"
    );
    assert!(columns.columnoptionals.is_some(), "include-not-null");
    let defaults = columns.columndefaults.as_deref().expect("include-default");
    assert!(
        defaults.iter().any(Option::is_none),
        "a column with no DEFAULT"
    );
    assert!(
        defaults.iter().any(Option::is_some),
        "a column with a DEFAULT"
    );
    assert!(pk.is_some(), "include-pk");

    let oldkeys = tx
        .change
        .iter()
        .find_map(|change| match change {
            ChangeV1::Update { oldkeys, .. } => Some(oldkeys),
            _ => None,
        })
        .expect("an update");
    assert!(oldkeys.keytypes.is_some(), "keytypes");
    assert!(oldkeys.keytypeoids.is_some(), "keytypeoids");

    assert!(
        tx.change
            .iter()
            .any(|change| matches!(change, ChangeV1::Delete { .. })),
        "a delete"
    );
    assert!(
        tx.change.iter().any(|change| {
            matches!(change, ChangeV1::Message { transactional, .. } if *transactional)
        }),
        "a transactional message inside the transaction"
    );
}
