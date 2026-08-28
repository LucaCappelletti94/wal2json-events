#![allow(missing_docs)]

use serde_json::{Value, json};
use wal2json_events::{
    Action, ChangeV1, Column, ColumnArrays, MessageV2, OldKeys, ParseError, PrimaryKeyV1,
    TransactionV1, parse_v1, parse_v1_lines, parse_v1_slice, parse_v2, parse_v2_lines,
    parse_v2_slice,
};

const V1_FIXTURE: &str = include_str!("fixtures/wal2json-v1.json");
const V2_FIXTURE: &str = include_str!("fixtures/wal2json-v2.jsonl");
const MISSING_TABLE_FIXTURE: &str = include_str!("fixtures/missing-table-v2.json");

fn triples(columns: &[Column]) -> Vec<(&str, Option<&str>, Option<&Value>)> {
    columns
        .iter()
        .map(|column| {
            (
                column.name.as_str(),
                column.type_name.as_deref(),
                column.value.as_ref(),
            )
        })
        .collect()
}

fn array_triples(arrays: &ColumnArrays) -> Vec<(&str, Option<&str>, Option<&Value>)> {
    arrays
        .columnnames
        .iter()
        .enumerate()
        .map(|(i, name)| {
            (
                name.as_str(),
                arrays.columntypes.as_ref().map(|types| types[i].as_str()),
                Some(&arrays.columnvalues[i]),
            )
        })
        .collect()
}

fn key_triples(keys: &OldKeys) -> Vec<(&str, Option<&str>, Option<&Value>)> {
    keys.keynames
        .iter()
        .enumerate()
        .map(|(i, name)| {
            (
                name.as_str(),
                keys.keytypes.as_ref().map(|types| types[i].as_str()),
                Some(&keys.keyvalues[i]),
            )
        })
        .collect()
}

#[test]
fn v1_v2_parity() {
    let v1_tx = parse_v1(V1_FIXTURE).unwrap();
    let v2_msgs: Vec<_> = V2_FIXTURE.lines().map(|l| parse_v2(l).unwrap()).collect();

    let v2_dml: Vec<_> = v2_msgs
        .iter()
        .filter(|m| m.action() != Action::Truncate)
        .collect();
    assert_eq!(v1_tx.change.len(), v2_dml.len());

    for (i, (v1, v2)) in v1_tx.change.iter().zip(v2_dml.iter()).enumerate() {
        assert_eq!(v1.table(), v2.table(), "table mismatch at {i}");
        assert_eq!(v1.schema(), v2.schema(), "schema mismatch at {i}");

        match (v1, v2) {
            (ChangeV1::Insert { columns, .. }, MessageV2::Insert(row)) => {
                assert_eq!(
                    array_triples(columns),
                    triples(row.columns.as_deref().unwrap()),
                    "insert columns mismatch at {i}"
                );
            }
            (
                ChangeV1::Update {
                    columns, oldkeys, ..
                },
                MessageV2::Update(row),
            ) => {
                assert_eq!(
                    array_triples(columns),
                    triples(row.columns.as_deref().unwrap()),
                    "update columns mismatch at {i}"
                );
                assert_eq!(
                    key_triples(oldkeys),
                    triples(row.identity.as_deref().unwrap()),
                    "update identity mismatch at {i}"
                );
            }
            (ChangeV1::Delete { oldkeys, .. }, MessageV2::Delete(row)) => {
                assert_eq!(
                    key_triples(oldkeys),
                    triples(row.identity.as_deref().unwrap()),
                    "delete identity mismatch at {i}"
                );
            }
            (v1, v2) => panic!("kind mismatch at {i}: {v1:?} against {v2:?}"),
        }
    }
}

#[test]
fn roundtrip_message_v2_corpus() {
    for line in V2_FIXTURE.lines() {
        let original = parse_v2(line).unwrap();
        let json = serde_json::to_string(&original).unwrap();
        let reparsed = parse_v2(&json).unwrap();
        assert_eq!(original, reparsed, "roundtrip mismatch for line: {line}");
    }
}

#[test]
fn roundtrip_transaction_v1() {
    let original = parse_v1(V1_FIXTURE).unwrap();
    let json = serde_json::to_string(&original).unwrap();
    let reparsed = parse_v1(&json).unwrap();
    assert_eq!(original, reparsed);
}

#[test]
fn roundtrip_column() {
    let mut col = Column::new("x");
    col.type_name = Some("integer".into());
    col.typeoid = Some(23);
    col.value = Some(json!(42));
    col.optional = Some(false);
    col.position = Some(1);
    col.default = Some(Some("nextval('s'::regclass)".into()));
    let json = serde_json::to_string(&col).unwrap();
    let reparsed: Column = serde_json::from_str(&json).unwrap();
    assert_eq!(col, reparsed);
}

#[test]
fn roundtrip_old_keys() {
    let mut keys = OldKeys::new([
        ("id".to_owned(), json!(1)),
        ("code".to_owned(), json!("abc")),
    ]);
    keys.keytypes = Some(vec!["integer".into(), "text".into()]);
    keys.keytypeoids = Some(vec![23, 25]);
    let json = serde_json::to_string(&keys).unwrap();
    let reparsed: OldKeys = serde_json::from_str(&json).unwrap();
    assert_eq!(keys, reparsed);
}

#[test]
fn roundtrip_all_action_wire_letters() {
    let cases: &[(Action, &str)] = &[
        (Action::Begin, "\"B\""),
        (Action::Commit, "\"C\""),
        (Action::Insert, "\"I\""),
        (Action::Update, "\"U\""),
        (Action::Delete, "\"D\""),
        (Action::Truncate, "\"T\""),
        (Action::Message, "\"M\""),
    ];
    for (action, wire) in cases {
        let serialized = serde_json::to_string(action).unwrap();
        assert_eq!(&serialized, wire, "wrong wire letter for {action:?}");
        let reparsed: Action = serde_json::from_str(&serialized).unwrap();
        assert_eq!(&reparsed, action, "roundtrip failed for {action:?}");
    }
}

#[test]
fn row_action_without_table_is_error() {
    let err = parse_v2(MISSING_TABLE_FIXTURE).unwrap_err();

    assert!(
        matches!(
            err,
            ParseError::MissingField {
                field: "table",
                context: "a v2 row action"
            }
        ),
        "expected a typed missing-field error, got: {err:?}"
    );
    assert_eq!(
        err.to_string(),
        "a v2 row action requires the field `table`"
    );
}

#[test]
fn malformed_json_is_reported_as_such() {
    assert!(matches!(
        parse_v2("{\"action\":").unwrap_err(),
        ParseError::Json(_)
    ));
    assert!(matches!(
        parse_v1("not json at all").unwrap_err(),
        ParseError::Json(_)
    ));
    // A field of the wrong JSON type is serde's business, not a structural error.
    assert!(matches!(
        parse_v2(r#"{"action":"I","table":7}"#).unwrap_err(),
        ParseError::Json(_)
    ));
}

/// Validation lives in one place but has two entry points: the parse functions, which return a
/// typed error, and the `Deserialize` impls, which serde requires to produce its own error type.
/// Both must enforce the same rules.
#[test]
fn the_serde_path_enforces_the_same_rules() {
    let err = serde_json::from_str::<MessageV2>(r#"{"action":"I"}"#).unwrap_err();
    assert!(
        err.to_string().contains("requires the field `table`"),
        "got: {err}"
    );

    let err = serde_json::from_str::<Vec<MessageV2>>(r#"[{"action":"T"}]"#).unwrap_err();
    assert!(
        err.to_string().contains("requires the field `table`"),
        "got: {err}"
    );

    let err = serde_json::from_str::<ChangeV1>(
        r#"{"kind":"insert","table":"t","columnnames":["a"],"columnvalues":[]}"#,
    )
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("`columnvalues` has length 0 but `columnnames` has length 1"),
        "got: {err}"
    );
}

#[test]
fn transaction_boundaries_carry_no_required_field() {
    for json in [r#"{"action":"B"}"#, r#"{"action":"C"}"#] {
        assert!(parse_v2(json).is_ok(), "expected Ok for: {json}");
    }
}

/// The wire letters live in two places: the [`Action`] renames that drive deserialization, and the
/// `MessageV2` variant tags that drive serialization. This pins them to each other.
#[test]
fn every_action_serializes_back_to_its_wire_letter() {
    let cases = [
        ("B", None, r#"{"action":"B"}"#),
        ("C", None, r#"{"action":"C"}"#),
        ("I", Some("t"), r#"{"action":"I","schema":"s","table":"t"}"#),
        ("U", Some("t"), r#"{"action":"U","schema":"s","table":"t"}"#),
        ("D", Some("t"), r#"{"action":"D","schema":"s","table":"t"}"#),
        ("T", Some("t"), r#"{"action":"T","schema":"s","table":"t"}"#),
        (
            "M",
            None,
            r#"{"action":"M","transactional":true,"prefix":"p","content":"c"}"#,
        ),
    ];
    for (letter, table, json) in cases {
        let parsed = parse_v2(json).unwrap();
        let reserialized: Value = serde_json::to_value(&parsed).unwrap();

        assert_eq!(reserialized["action"], json!(letter), "for input {json}");
        assert_eq!(
            serde_json::to_value(parsed.action()).unwrap(),
            json!(letter),
            "Action letter disagrees with the variant tag for {json}"
        );
        assert_eq!(reserialized, serde_json::from_str::<Value>(json).unwrap());

        // The accessors answer for every action, including the ones that have no table.
        assert_eq!(parsed.table(), table, "table() for {json}");
        assert_eq!(parsed.schema(), table.map(|_| "s"), "schema() for {json}");
    }
}

#[test]
fn duplicate_action_field_is_error() {
    let json = r#"{"action":"I","action":"U","schema":"public","table":"t","columns":[]}"#;
    assert!(
        parse_v2(json).is_err(),
        "duplicate action field must be rejected"
    );
}

#[test]
fn explicit_null_optional_fields_parse_as_none() {
    let json = r#"{"action":"I","schema":null,"table":"t","columns":null,"identity":null,"lsn":null,"xid":null}"#;
    let MessageV2::Insert(row) = parse_v2(json).unwrap() else {
        panic!("expected an insert");
    };
    assert_eq!(row.schema, None);
    assert_eq!(row.columns, None);
    assert_eq!(row.identity, None);
    assert_eq!(row.lsn, None);
    assert_eq!(row.xid, None);
}

// E1: a transaction carrying a logical message used to fail on the missing `schema` field.

#[test]
fn v1_logical_message_change_parses() {
    let json = r#"{"change":[{"kind":"message","transactional":true,"prefix":"myapp","content":"hello"}]}"#;
    let tx = parse_v1(json).unwrap();

    assert_eq!(
        tx.change,
        [ChangeV1::Message {
            transactional: true,
            prefix: "myapp".to_owned(),
            content: "hello".to_owned(),
        }]
    );
}

#[test]
fn v1_mixed_transaction_with_message_parses() {
    let json = r#"{"xid":749,"change":[{"kind":"insert","schema":"public","table":"t","columnnames":["id"],"columntypes":["integer"],"columnvalues":[1]},{"kind":"message","transactional":true,"prefix":"p","content":"c"}]}"#;
    let tx = parse_v1(json).unwrap();

    assert_eq!(tx.xid, Some(749));
    assert_eq!(tx.change.len(), 2);
    assert!(matches!(tx.change[1], ChangeV1::Message { .. }));
}

#[test]
fn v1_change_without_schema_parses() {
    // include-schemas=false omits the schema from every change.
    let json =
        r#"{"change":[{"kind":"insert","table":"t","columnnames":["id"],"columnvalues":[1]}]}"#;
    let tx = parse_v1(json).unwrap();

    assert_eq!(tx.change[0].schema(), None);
    assert_eq!(tx.change[0].table(), Some("t"));
}

#[test]
fn v2_column_without_type_parses() {
    // include-types=false omits the type from every column.
    let json =
        r#"{"action":"I","schema":"public","table":"t","columns":[{"name":"id","value":1}]}"#;
    let MessageV2::Insert(row) = parse_v2(json).unwrap() else {
        panic!("expected an insert");
    };
    let columns = row.columns.unwrap();

    assert_eq!(columns[0].type_name, None);
    assert_eq!(columns[0].value, Some(json!(1)));
}

#[test]
fn v2_primary_key_entry_without_value_parses() {
    // include-pk=true emits identities with no value at all.
    let json = r#"{"action":"I","schema":"public","table":"t","columns":[{"name":"id","type":"integer","value":1}],"pk":[{"name":"id","type":"integer"}]}"#;
    let MessageV2::Insert(row) = parse_v2(json).unwrap() else {
        panic!("expected an insert");
    };
    let pk = row.pk.unwrap();

    assert_eq!(pk[0].name, "id");
    assert_eq!(pk[0].value, None);
}

#[test]
fn v2_null_column_value_stays_distinct_from_an_absent_one() {
    let json = r#"{"action":"I","schema":"public","table":"t","columns":[{"name":"a","value":null},{"name":"b"}]}"#;
    let MessageV2::Insert(row) = parse_v2(json).unwrap() else {
        panic!("expected an insert");
    };
    let columns = row.columns.unwrap();

    assert_eq!(columns[0].value, Some(Value::Null), "SQL NULL");
    assert_eq!(columns[1].value, None, "no value key at all");
}

#[test]
fn v2_null_column_default_stays_distinct_from_an_absent_one() {
    let json = r#"{"action":"I","schema":"public","table":"t","columns":[{"name":"a","value":1,"default":null},{"name":"b","value":2}]}"#;
    let MessageV2::Insert(row) = parse_v2(json).unwrap() else {
        panic!("expected an insert");
    };
    let columns = row.columns.unwrap();

    assert_eq!(columns[0].default, Some(None), "no DEFAULT clause");
    assert_eq!(columns[1].default, None, "include-default was off");
}

// E2: fields that only appear under a plugin option used to be dropped.

#[test]
fn v2_transaction_boundaries_keep_their_fields() {
    let begin = r#"{"action":"B","xid":749,"timestamp":"2026-08-28 12:34:56.789+05:30","origin":0,"lsn":"0/1A2B3C0","nextlsn":"0/1A2B3C8"}"#;
    let MessageV2::Begin(boundary) = parse_v2(begin).unwrap() else {
        panic!("expected a begin");
    };

    assert_eq!(boundary.xid, Some(749));
    assert_eq!(
        boundary.timestamp.as_deref(),
        Some("2026-08-28 12:34:56.789+05:30")
    );
    assert_eq!(boundary.origin, Some(0));
    assert_eq!(boundary.lsn.as_deref(), Some("0/1A2B3C0"));
    assert_eq!(boundary.nextlsn.as_deref(), Some("0/1A2B3C8"));
}

#[test]
fn v2_logical_message_keeps_its_payload() {
    let json = r#"{"action":"M","xid":null,"timestamp":null,"origin":null,"transactional":false,"prefix":"myapp","content":"hello"}"#;
    let MessageV2::Message(message) = parse_v2(json).unwrap() else {
        panic!("expected a message");
    };

    assert!(!message.transactional);
    assert_eq!(message.prefix, "myapp");
    assert_eq!(message.content, "hello");
    assert_eq!(message.xid, None);
}

#[test]
fn v2_logical_message_without_its_payload_is_error() {
    for json in [
        r#"{"action":"M"}"#,
        r#"{"action":"M","transactional":true}"#,
        r#"{"action":"M","transactional":true,"prefix":"p"}"#,
    ] {
        assert!(parse_v2(json).is_err(), "expected Err for: {json}");
    }
}

#[test]
fn v2_row_keeps_its_option_driven_fields() {
    let json = r#"{"action":"U","xid":749,"timestamp":"2026-08-28 12:34:56+05:30","origin":0,"lsn":"0/1A2B3C0","schema":"public","table":"t","columns":[{"name":"id","type":"integer","typeoid":23,"value":1,"optional":false,"position":1,"default":null}],"identity":[{"name":"id","type":"integer","value":1}],"pk":[{"name":"id","type":"integer"}]}"#;
    let MessageV2::Update(row) = parse_v2(json).unwrap() else {
        panic!("expected an update");
    };

    assert_eq!(row.xid, Some(749));
    assert_eq!(row.origin, Some(0));
    assert_eq!(row.lsn.as_deref(), Some("0/1A2B3C0"));
    assert!(row.pk.is_some());

    let column = &row.columns.as_ref().unwrap()[0];
    assert_eq!(column.typeoid, Some(23));
    assert_eq!(column.optional, Some(false));
    assert_eq!(column.position, Some(1));
    assert_eq!(column.default, Some(None));

    // Nothing was dropped, so the round trip reproduces the input exactly.
    let expected: Value = serde_json::from_str(json).unwrap();
    assert_eq!(
        serde_json::to_value(parse_v2(json).unwrap()).unwrap(),
        expected
    );
}

#[test]
fn v1_transaction_keeps_its_option_driven_fields() {
    let json = r#"{"xid":749,"nextlsn":"0/1A2B3C8","timestamp":"2026-08-28 12:34:56+05:30","origin":0,"change":[{"kind":"insert","schema":"public","table":"t","columnnames":["id"],"columntypes":["integer"],"columntypeoids":[23],"columnpositions":[1],"columnoptionals":[false],"columndefaults":[null],"columnvalues":[1],"pk":{"pknames":["id"],"pktypes":["integer"]}}]}"#;
    let tx = parse_v1(json).unwrap();

    assert_eq!(tx.xid, Some(749));
    assert_eq!(tx.nextlsn.as_deref(), Some("0/1A2B3C8"));
    assert_eq!(tx.timestamp.as_deref(), Some("2026-08-28 12:34:56+05:30"));
    assert_eq!(tx.origin, Some(0));

    let ChangeV1::Insert { columns, pk, .. } = &tx.change[0] else {
        panic!("expected an insert");
    };
    assert_eq!(columns.columntypeoids.as_deref(), Some([23].as_slice()));
    assert_eq!(columns.columnpositions.as_deref(), Some([1].as_slice()));
    assert_eq!(columns.columnoptionals.as_deref(), Some([false].as_slice()));
    assert_eq!(columns.columndefaults.as_deref(), Some([None].as_slice()));
    let mut expected_pk = PrimaryKeyV1::new(vec!["id".to_owned()]);
    expected_pk.pktypes = vec!["integer".to_owned()];
    assert_eq!(pk.as_ref().unwrap(), &expected_pk);

    let expected: Value = serde_json::from_str(json).unwrap();
    assert_eq!(serde_json::to_value(tx).unwrap(), expected);
}

// E3: co-indexed arrays of differing lengths used to parse as Ok.

#[test]
fn v1_mismatched_column_arrays_are_error() {
    let cases = [
        (
            "columnvalues",
            1,
            r#"{"change":[{"kind":"insert","table":"t","columnnames":["a","b","c"],"columnvalues":[1]}]}"#,
        ),
        (
            "columntypes",
            0,
            r#"{"change":[{"kind":"insert","table":"t","columnnames":["a"],"columntypes":[],"columnvalues":[1]}]}"#,
        ),
        (
            "columntypeoids",
            2,
            r#"{"change":[{"kind":"insert","table":"t","columnnames":["a"],"columnvalues":[1],"columntypeoids":[23,25]}]}"#,
        ),
        (
            "columnpositions",
            0,
            r#"{"change":[{"kind":"insert","table":"t","columnnames":["a"],"columnvalues":[1],"columnpositions":[]}]}"#,
        ),
        (
            "columnoptionals",
            2,
            r#"{"change":[{"kind":"insert","table":"t","columnnames":["a"],"columnvalues":[1],"columnoptionals":[true,false]}]}"#,
        ),
        (
            "columndefaults",
            0,
            r#"{"change":[{"kind":"insert","table":"t","columnnames":["a"],"columnvalues":[1],"columndefaults":[]}]}"#,
        ),
    ];
    for (name, length, json) in cases {
        match parse_v1(json).unwrap_err() {
            ParseError::LengthMismatch {
                field,
                len,
                reference,
                ..
            } => {
                assert_eq!(field, name, "wrong field named for {json}");
                assert_eq!(len, length, "wrong length reported for {json}");
                assert_eq!(reference, "columnnames");
            }
            other => panic!("expected a length mismatch for {json}, got: {other:?}"),
        }
    }
}

#[test]
fn v1_mismatched_old_keys_are_error() {
    let cases = [
        (
            "keyvalues",
            r#"{"change":[{"kind":"delete","table":"t","oldkeys":{"keynames":["a","b"],"keyvalues":[1]}}]}"#,
        ),
        (
            "keytypes",
            r#"{"change":[{"kind":"delete","table":"t","oldkeys":{"keynames":["a"],"keytypes":["integer","text"],"keyvalues":[1]}}]}"#,
        ),
        (
            "keytypeoids",
            r#"{"change":[{"kind":"delete","table":"t","oldkeys":{"keynames":["a"],"keytypeoids":[],"keyvalues":[1]}}]}"#,
        ),
    ];
    for (name, json) in cases {
        match parse_v1(json).unwrap_err() {
            ParseError::LengthMismatch {
                field, reference, ..
            } => {
                assert_eq!(field, name, "wrong field named for {json}");
                assert_eq!(reference, "keynames");
            }
            other => panic!("expected a length mismatch for {json}, got: {other:?}"),
        }
    }
}

#[test]
fn v1_mismatched_primary_key_arrays_are_error() {
    let json = r#"{"change":[{"kind":"insert","table":"t","columnnames":["a"],"columnvalues":[1],"pk":{"pknames":["a","b"],"pktypes":["integer"]}}]}"#;
    let err = parse_v1(json).unwrap_err();

    assert!(
        matches!(
            err,
            ParseError::LengthMismatch {
                field: "pktypes",
                len: 1,
                reference: "pknames",
                expected: 2
            }
        ),
        "got: {err:?}"
    );
}

#[test]
fn v1_empty_primary_key_types_are_accepted() {
    // wal2json emits the pktypes key even under include-types=false, and then it is empty.
    let json = r#"{"change":[{"kind":"insert","table":"t","columnnames":["a"],"columnvalues":[1],"pk":{"pknames":["a"],"pktypes":[]}}]}"#;
    let tx = parse_v1(json).unwrap();

    let ChangeV1::Insert { pk, .. } = &tx.change[0] else {
        panic!("expected an insert");
    };
    assert_eq!(pk.as_ref().unwrap().pktypes, [] as [String; 0]);
}

// E4: absent fields used to serialize as explicit nulls.

#[test]
fn serialization_omits_absent_fields() {
    let begin = parse_v2(r#"{"action":"B"}"#).unwrap();
    assert_eq!(serde_json::to_string(&begin).unwrap(), r#"{"action":"B"}"#);

    let delete = parse_v2(
        r#"{"action":"D","schema":"public","table":"t","identity":[{"name":"id","type":"integer","value":1}]}"#,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_string(&delete).unwrap(),
        r#"{"action":"D","schema":"public","table":"t","identity":[{"name":"id","type":"integer","value":1}]}"#
    );

    let tx = parse_v1(
        r#"{"change":[{"kind":"insert","schema":"public","table":"t","columnnames":["id"],"columntypes":["integer"],"columnvalues":[1]}]}"#,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_string(&tx).unwrap(),
        r#"{"change":[{"kind":"insert","schema":"public","table":"t","columnnames":["id"],"columntypes":["integer"],"columnvalues":[1]}]}"#
    );
}

// E5: PostgreSQL numeric precision beyond f64.

const NUMERIC: &str = r#"{"action":"I","schema":"public","table":"t","columns":[{"name":"amount","type":"numeric","value":12345678901234567890.123456789}]}"#;

fn parse_numeric() -> Value {
    let MessageV2::Insert(row) = parse_v2(NUMERIC).unwrap() else {
        panic!("expected an insert");
    };
    row.columns.unwrap().swap_remove(0).value.unwrap()
}

#[cfg(feature = "arbitrary_precision")]
#[test]
fn numeric_keeps_full_precision() {
    assert_eq!(
        serde_json::to_string(&parse_numeric()).unwrap(),
        "12345678901234567890.123456789"
    );
}

#[cfg(not(feature = "arbitrary_precision"))]
#[test]
fn numeric_loses_precision_without_the_feature() {
    assert_ne!(
        serde_json::to_string(&parse_numeric()).unwrap(),
        "12345678901234567890.123456789"
    );
}

/// A value the crate handed back must not change when it is handed in again. The default
/// `serde_json` float parser cannot read back the text `serde_json` itself writes, which shifts a
/// `numeric` in its last bit, so the crate enables `float_roundtrip`. Dropping it fails this test.
///
/// This exact value came from a fuzz run: its shortest float text form reads back as a neighbouring
/// float under the fast parser.
#[test]
fn numeric_survives_a_round_trip() {
    let json = r#"{"action":"I","schema":"public","table":"t","columns":[{"name":"amount","type":"numeric","value":12345678902134567890.123456789}]}"#;
    let parsed = parse_v2(json).unwrap();

    let reparsed = parse_v2(&serde_json::to_string(&parsed).unwrap()).unwrap();

    assert_eq!(parsed, reparsed);
}

/// The enums are exhaustive on purpose, so a consumer can match every variant without a catch-all
/// and hear from the compiler if a wal2json release ever adds an action letter. Marking any of them
/// `#[non_exhaustive]` again would stop this test compiling, which is the point of it.
#[test]
fn the_enums_can_be_matched_exhaustively() {
    fn letter(message: &MessageV2) -> &'static str {
        match message {
            MessageV2::Begin(_) => "B",
            MessageV2::Commit(_) => "C",
            MessageV2::Insert(_) => "I",
            MessageV2::Update(_) => "U",
            MessageV2::Delete(_) => "D",
            MessageV2::Truncate(_) => "T",
            MessageV2::Message(_) => "M",
        }
    }

    fn kind(change: &ChangeV1) -> &'static str {
        match change {
            ChangeV1::Insert { .. } => "insert",
            ChangeV1::Update { .. } => "update",
            ChangeV1::Delete { .. } => "delete",
            ChangeV1::Message { .. } => "message",
        }
    }

    fn wire(action: Action) -> &'static str {
        match action {
            Action::Begin => "B",
            Action::Commit => "C",
            Action::Insert => "I",
            Action::Update => "U",
            Action::Delete => "D",
            Action::Truncate => "T",
            Action::Message => "M",
        }
    }

    let truncate = parse_v2(r#"{"action":"T","schema":"public","table":"t"}"#).unwrap();
    assert_eq!(letter(&truncate), "T");
    assert_eq!(wire(truncate.action()), "T");

    let message = parse_v1(
        r#"{"change":[{"kind":"message","transactional":true,"prefix":"p","content":"c"}]}"#,
    )
    .unwrap();
    assert_eq!(kind(&message.change[0]), "message");
}

/// A wal2json stream is line delimited, so the crate walks it rather than making every caller
/// write the same loop.
#[test]
fn a_stream_is_parsed_line_by_line() {
    let messages: Vec<MessageV2> = parse_v2_lines(V2_FIXTURE).map(Result::unwrap).collect();
    assert_eq!(messages.len(), V2_FIXTURE.lines().count());
    assert_eq!(messages[0].table(), Some("default_rows"));

    // A trailing newline, and a blank line inside the stream, are not errors.
    let padded = format!("{V2_FIXTURE}\n\n");
    assert_eq!(parse_v2_lines(&padded).count(), messages.len());

    let transactions: Vec<TransactionV1> = parse_v1_lines(V1_FIXTURE).map(Result::unwrap).collect();
    assert_eq!(transactions.len(), 1);
    assert_eq!(transactions[0].change.len(), 7);
}

/// One bad line does not end the iteration, so a consumer can log it and carry on.
#[test]
fn a_stream_reports_each_line_separately() {
    let stream = format!(
        "{}\n{{\"action\":\"I\"}}\n{}",
        r#"{"action":"B"}"#, r#"{"action":"C"}"#
    );
    let results: Vec<_> = parse_v2_lines(&stream).collect();

    assert_eq!(results.len(), 3);
    assert!(results[0].is_ok());
    assert!(matches!(
        results[1],
        Err(ParseError::MissingField { field: "table", .. })
    ));
    assert!(results[2].is_ok());
}

#[test]
fn bytes_parse_the_same_as_text() {
    for line in V2_FIXTURE.lines() {
        assert_eq!(
            parse_v2_slice(line.as_bytes()).unwrap(),
            parse_v2(line).unwrap()
        );
    }
    assert_eq!(
        parse_v1_slice(V1_FIXTURE.as_bytes()).unwrap(),
        parse_v1(V1_FIXTURE).unwrap()
    );
}
