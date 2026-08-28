# wal2json-events

[![crates.io](https://img.shields.io/crates/v/wal2json-events.svg)](https://crates.io/crates/wal2json-events)
[![docs.rs](https://img.shields.io/docsrs/wal2json-events)](https://docs.rs/wal2json-events)
[![CI](https://github.com/LucaCappelletti94/wal2json-events/actions/workflows/ci.yml/badge.svg)](https://github.com/LucaCappelletti94/wal2json-events/actions/workflows/ci.yml)
[![MSRV](https://img.shields.io/badge/MSRV-1.85-blue.svg)](https://github.com/LucaCappelletti94/wal2json-events)
[![license](https://img.shields.io/crates/l/wal2json-events.svg)](https://github.com/LucaCappelletti94/wal2json-events/blob/main/LICENSE)

Typed parsers for PostgreSQL wal2json logical decoding output, format versions 1 and 2.

Each format is an enum over the wire's own discriminator, so a field only exists on the events that carry it: a v2 row always has a table, a logical message always has its prefix and content.

```rust
use wal2json_events::{MessageV2, parse_v2};

let json = r#"{"action":"I","schema":"public","table":"users","columns":[{"name":"id","type":"integer","value":1}]}"#;

let MessageV2::Insert(row) = parse_v2(json)? else {
    panic!("expected an insert");
};
assert_eq!(row.table, "users");
assert_eq!(row.schema.as_deref(), Some("public"));

let columns = row.columns.unwrap();
assert_eq!(columns[0].name, "id");
assert_eq!(columns[0].value, Some(serde_json::json!(1)));
# Ok::<(), wal2json_events::ParseError>(())
```

Every field wal2json can emit is modelled, including option-gated ones such as `include-xids`, `include-lsn`, `include-pk` and `include-type-oids`. A field the running configuration omits is `None`, and re-serializing omits it again rather than writing a null. Both directions are tested against captured PostgreSQL output, with the options off and with all of them on.

Format version 1 wraps a transaction in one object and describes rows as parallel arrays. `parse_v1` rejects arrays that disagree in length, so anything it returns can be zipped.

```rust
use wal2json_events::{ChangeV1, parse_v1};

let json = r#"{"xid":749,"change":[{"kind":"insert","schema":"public","table":"users",
    "columnnames":["id","email"],"columntypes":["integer","text"],"columnvalues":[7,"a@b.c"]}]}"#;

let transaction = parse_v1(json)?;
assert_eq!(transaction.xid, Some(749));

let ChangeV1::Insert { table, columns, .. } = &transaction.change[0] else {
    panic!("expected an insert");
};
assert_eq!(table, "users");

let named: Vec<(&str, &serde_json::Value)> = columns
    .columnnames
    .iter()
    .map(String::as_str)
    .zip(&columns.columnvalues)
    .collect();
assert_eq!(named[1].0, "email");
# Ok::<(), wal2json_events::ParseError>(())
```

A v2 stream is one message per line. `parse_v2_lines` yields a result per line, so a bad line can be logged rather than ending the stream, and `parse_v2_slice` takes bytes. `parse_v1_lines` and `parse_v1_slice` match, over whole transactions.

```rust
use wal2json_events::{Action, parse_v2_lines};

let stream = "\
{\"action\":\"B\",\"xid\":749}
{\"action\":\"I\",\"schema\":\"public\",\"table\":\"users\",\"columns\":[]}
{\"action\":\"C\",\"xid\":749}
";

let actions: Vec<Action> = parse_v2_lines(stream)
    .map(|message| message.map(|message| message.action()))
    .collect::<Result<_, _>>()?;

assert_eq!(actions, [Action::Begin, Action::Insert, Action::Commit]);
# Ok::<(), wal2json_events::ParseError>(())
```

`ParseError` separates malformed JSON from the two rules the model enforces, a required field and co-indexed array lengths, and names the wire field involved.

```rust
use wal2json_events::{ParseError, parse_v2};

let err = parse_v2(r#"{"action":"I","schema":"public"}"#).unwrap_err();

assert!(matches!(err, ParseError::MissingField { field: "table", .. }));
assert_eq!(err.to_string(), "a v2 row action requires the field `table`");
```

The data structures are `#[non_exhaustive]`, so a new wal2json field is not a breaking change. Build values with `Column::new("id")`, `RowV2::new`, `OldKeys::new` and assign the optional fields after. `ColumnArrays::new` and `OldKeys::new` take name and value pairs, making a length mismatch unbuildable. The enums are exhaustive, so a `match` needs no catch-all and a future action letter becomes a compile error.

A `bytea` arrives hex encoded with wal2json's leading `\x` already stripped, so `deadbeef` is four bytes. This holds only under the default `bytea_output = 'hex'`: with `escape`, wal2json strips two significant characters instead and the value is already corrupt.

Column values are `serde_json::Value`. A `numeric` wider than an `f64` loses digits but will not drift, since the crate enables `serde_json`'s `float_roundtrip`, without which the parser cannot read back the text it wrote. For exact decimals:

```toml
wal2json-events = { version = "0.1", features = ["arbitrary_precision"] }
```
