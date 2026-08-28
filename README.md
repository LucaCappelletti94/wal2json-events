# wal2json-events

[![crates.io](https://img.shields.io/crates/v/wal2json-events.svg)](https://crates.io/crates/wal2json-events)
[![docs.rs](https://img.shields.io/docsrs/wal2json-events)](https://docs.rs/wal2json-events)
[![CI](https://github.com/LucaCappelletti94/wal2json-events/actions/workflows/ci.yml/badge.svg)](https://github.com/LucaCappelletti94/wal2json-events/actions/workflows/ci.yml)
[![MSRV](https://img.shields.io/badge/MSRV-1.85-blue.svg)](https://github.com/LucaCappelletti94/wal2json-events)
[![license](https://img.shields.io/crates/l/wal2json-events.svg)](https://github.com/LucaCappelletti94/wal2json-events/blob/main/LICENSE)

`wal2json-events` parses PostgreSQL wal2json logical decoding output into typed Rust structures for format versions 1 and 2.

Both formats are modelled as an enum over the wire's own discriminator, so a field only exists on the events that actually carry it. A v2 row event always has a table, a transaction boundary always has room for its transaction id, and a logical message carries its prefix and content.

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

Every field wal2json can emit is represented, including the ones that only appear under a plugin option such as `include-xids`, `include-lsn`, `include-pk` or `include-type-oids`. A field that the running configuration does not emit is `None` rather than lost, and re-serializing a parsed event omits it again instead of writing a null. Both halves of that are tested against output captured from PostgreSQL with wal2json, once with the options off and once with every field-adding option on: each captured line parses, and serializing it back reproduces the captured JSON.

Format version 1 wraps a whole transaction in one object, and describes a row with arrays that run in parallel. `parse_v1` rejects input whose arrays disagree in length, so any value it returns can be zipped safely.

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

A v2 stream carries one message per line, so `parse_v2_lines` walks it and yields a result per message, which lets a consumer log one bad line and carry on. `parse_v2_slice` takes a line still held as bytes. `parse_v1_lines` and `parse_v1_slice` are the same for v1, whose lines are whole transactions.

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

One value needs care beyond its type. A `bytea` column arrives hex encoded with the leading `\x` already stripped by wal2json, so `deadbeef` means those four bytes. That holds only while the session reading the replication slot uses the default `bytea_output = 'hex'`: under `escape` wal2json still removes two leading characters, which corrupts the value before this crate ever sees it.

Both parse functions return `ParseError`, which separates malformed JSON from the two structural rules the wire model enforces: a field the action or kind requires, and those arrays agreeing in length. Each carries the wire name of the field involved, so a caller can react to a specific violation instead of matching on message text.

```rust
use wal2json_events::{ParseError, parse_v2};

let err = parse_v2(r#"{"action":"I","schema":"public"}"#).unwrap_err();

assert!(matches!(
    err,
    ParseError::MissingField {
        field: "table",
        ..
    }
));
assert_eq!(err.to_string(), "a v2 row action requires the field `table`");
```

Because wal2json keeps adding fields as it gains options, the data structures are marked `#[non_exhaustive]`: a new field is then an ordinary release rather than a breaking one. The price is that struct literals only work inside the crate, so build a value with `Column::new("id")` (or `RowV2::new`, `OldKeys::new`, and so on) and assign the optional fields afterwards. `ColumnArrays::new` and `OldKeys::new` take name and value pairs, which makes a length disagreement impossible to build in the first place. The enums are exhaustive, so a `match` on `MessageV2` or `ChangeV1` needs no catch-all arm and the compiler will tell you if a wal2json release ever adds an action letter.

Column values use `serde_json::Value`, with JSON numbers represented by `serde_json::Number`. A PostgreSQL `numeric` wider than an `f64` loses digits, though it will not drift further: the crate turns on `serde_json`'s `float_roundtrip` because the default float parser cannot read back the text `serde_json` itself writes, which would let a value change every time it passed through. When the exact decimal matters, enable the `arbitrary_precision` feature, which keeps the number as written:

```toml
wal2json-events = { version = "0.1", features = ["arbitrary_precision"] }
```
