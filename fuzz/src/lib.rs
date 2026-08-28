//! Invariants shared by the fuzz targets.

use serde_json::Value;
use wal2json_events::{ChangeV1, ColumnArrays, OldKeys, PrimaryKeyV1, TransactionV1};

/// Asserts serialization invents and corrupts nothing: every key the model emits was in the input
/// at the same path with an equal value, and no array changed length.
///
/// The converse, that no input field was dropped, is position dependent (a `B` ignores `table`), so
/// asserting it here would restate the model's field map. The captured corpora cover that.
pub fn assert_nothing_invented(input: &Value, output: &Value, path: &str) {
    match (input, output) {
        (Value::Object(input_fields), Value::Object(output_fields)) => {
            for (key, output_field) in output_fields {
                let input_field = input_fields.get(key).unwrap_or_else(|| {
                    panic!("serialized `{path}{key}`, which the input does not carry")
                });
                assert_nothing_invented(input_field, output_field, &format!("{path}{key}."));
            }
        }
        (Value::Array(input_items), Value::Array(output_items)) => {
            assert_eq!(
                input_items.len(),
                output_items.len(),
                "array length changed at `{path}`"
            );
            for (index, (input_item, output_item)) in
                input_items.iter().zip(output_items).enumerate()
            {
                assert_nothing_invented(input_item, output_item, &format!("{path}{index}."));
            }
        }
        (input_value, output_value) => {
            assert_eq!(input_value, output_value, "value changed at `{path}`");
        }
    }
}

/// Asserts the guarantee `parse_v1` documents: every array is co-indexed with its names array.
pub fn assert_co_indexed(transaction: &TransactionV1) {
    for change in &transaction.change {
        match change {
            ChangeV1::Insert { columns, pk, .. } => {
                assert_columns(columns);
                assert_pk(pk.as_ref());
            }
            ChangeV1::Update {
                columns,
                pk,
                oldkeys,
                ..
            } => {
                assert_columns(columns);
                assert_pk(pk.as_ref());
                assert_old_keys(oldkeys);
            }
            ChangeV1::Delete { pk, oldkeys, .. } => {
                assert_pk(pk.as_ref());
                assert_old_keys(oldkeys);
            }
            _ => {}
        }
    }
}

fn assert_columns(columns: &ColumnArrays) {
    let names = columns.columnnames.len();
    assert_eq!(columns.columnvalues.len(), names, "columnvalues");
    assert_length(
        columns.columntypes.as_deref().map(<[_]>::len),
        names,
        "columntypes",
    );
    assert_length(
        columns.columntypeoids.as_deref().map(<[_]>::len),
        names,
        "columntypeoids",
    );
    assert_length(
        columns.columnpositions.as_deref().map(<[_]>::len),
        names,
        "columnpositions",
    );
    assert_length(
        columns.columnoptionals.as_deref().map(<[_]>::len),
        names,
        "columnoptionals",
    );
    assert_length(
        columns.columndefaults.as_deref().map(<[_]>::len),
        names,
        "columndefaults",
    );
}

fn assert_old_keys(keys: &OldKeys) {
    let names = keys.keynames.len();
    assert_eq!(keys.keyvalues.len(), names, "keyvalues");
    assert_length(keys.keytypes.as_deref().map(<[_]>::len), names, "keytypes");
    assert_length(
        keys.keytypeoids.as_deref().map(<[_]>::len),
        names,
        "keytypeoids",
    );
}

fn assert_pk(pk: Option<&PrimaryKeyV1>) {
    if let Some(pk) = pk {
        // Empty passes: that is what include-types=false emits.
        if !pk.pktypes.is_empty() {
            assert_eq!(pk.pktypes.len(), pk.pknames.len(), "pktypes");
        }
    }
}

fn assert_length(actual: Option<usize>, expected: usize, field: &str) {
    if let Some(actual) = actual {
        assert_eq!(actual, expected, "{field}");
    }
}
