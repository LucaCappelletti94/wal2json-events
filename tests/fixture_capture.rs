#![allow(missing_docs)]
// Diesel's QueryableByName derive expands to `data: data`, blamed on the field below.
#![allow(clippy::redundant_field_names)]

use std::fs;
use std::str::FromStr;
use std::time::Duration;

use bigdecimal::BigDecimal;
use chrono::{TimeZone, Utc};
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use serde_json::{Value, json};
use testcontainers::core::{BuildImageOptions, IntoContainerPort, WaitFor};
use testcontainers::runners::{SyncBuilder, SyncRunner};
use testcontainers::{Container, GenericBuildableImage, GenericImage, ImageExt};
use wal2json_events::{parse_v1, parse_v2};

diesel::table! {
    default_rows (tenant_id, id) {
        tenant_id -> Int4,
        id -> Int4,
        payload -> Bytea,
        amount -> Numeric,
        observed_at -> Timestamptz,
        nullable_text -> Nullable<Text>,
        toasted_text -> Text,
        status -> Text,
    }
}

diesel::table! {
    full_rows (id) {
        id -> Int4,
        code -> Text,
        payload -> Text,
    }
}

diesel::table! {
    truncate_rows (id) {
        id -> Int4,
    }
}

diesel::table! {
    all_options_rows (id) {
        id -> Int4,
        label -> Text,
        amount -> Nullable<Numeric>,
        note -> Nullable<Text>,
    }
}

// Pinned once here and passed to the Dockerfile. The tag carries it because an existing tag skips
// the build, so a bump must produce a new image.
const WAL2JSON_VERSION: &str = "2.6-4.pgdg12+1";
const PG_IMAGE: &str = "wal2json-events/postgres-wal2json";
// Read at runtime, not include_str!, so the check sees the files as they are on disk.
const V1_PATH: &str = "tests/fixtures/wal2json-v1.json";
const V2_PATH: &str = "tests/fixtures/wal2json-v2.jsonl";
const V1_ALL_PATH: &str = "tests/fixtures/wal2json-v1-all-options.jsonl";
const V2_ALL_PATH: &str = "tests/fixtures/wal2json-v2-all-options.jsonl";
const MESSAGE_PREFIX: &str = "wal2json_events";

#[derive(QueryableByName)]
struct CapturedRow {
    #[diesel(sql_type = diesel::sql_types::Text)]
    data: String,
}

fn postgres() -> Container<GenericImage> {
    let tag = format!("16-wal2json-{}", WAL2JSON_VERSION.replace('+', "-"));
    let image = GenericBuildableImage::new(PG_IMAGE, tag)
        .with_dockerfile("tests/fixtures/Dockerfile.postgres")
        .build_image_with(
            BuildImageOptions::new()
                .with_skip_if_exists(true)
                .with_build_arg("WAL2JSON_VERSION", WAL2JSON_VERSION),
        )
        .expect("build PostgreSQL wal2json image");

    image
        .with_wait_for(WaitFor::message_on_stderr("ready to accept connections"))
        .with_exposed_port(5432.tcp())
        .with_env_var("POSTGRES_USER", "wal2json_test")
        .with_env_var("POSTGRES_PASSWORD", "wal2json_test")
        .with_env_var("POSTGRES_DB", "testdb")
        .with_cmd([
            "postgres",
            "-c",
            "wal_level=logical",
            "-c",
            "max_wal_senders=4",
            "-c",
            "max_replication_slots=4",
            "-c",
            "output_plugin_libraries=pgoutput,test_decoding,wal2json",
        ])
        .with_startup_timeout(Duration::from_secs(180))
        .start()
        .expect("start PostgreSQL")
}

fn connect(container: &Container<GenericImage>) -> PgConnection {
    let port = container
        .get_host_port_ipv4(5432.tcp())
        .expect("mapped PostgreSQL port");
    PgConnection::establish(&format!(
        "postgres://wal2json_test:wal2json_test@127.0.0.1:{port}/testdb"
    ))
    .expect("connect to PostgreSQL")
}

fn create_schema(connection: &mut PgConnection) {
    // Diesel has no typed DDL API for replica identity or column storage settings.
    connection
        .batch_execute(
            "SET TIME ZONE 'Asia/Kolkata';
             CREATE TABLE default_rows (
                 tenant_id integer NOT NULL,
                 id integer NOT NULL,
                 payload bytea NOT NULL,
                 amount numeric NOT NULL,
                 observed_at timestamptz NOT NULL,
                 nullable_text text,
                 toasted_text text NOT NULL,
                 status text NOT NULL,
                 PRIMARY KEY (tenant_id, id)
             );
             ALTER TABLE default_rows ALTER COLUMN toasted_text SET STORAGE EXTERNAL;
             CREATE TABLE full_rows (
                 id integer PRIMARY KEY,
                 code text NOT NULL,
                 payload text NOT NULL
             );
             ALTER TABLE full_rows REPLICA IDENTITY FULL;
             CREATE TABLE truncate_rows (id integer PRIMARY KEY);
             CREATE TABLE all_options_rows (
                 id integer PRIMARY KEY,
                 label text NOT NULL DEFAULT 'unset',
                 amount numeric,
                 note text
             );",
        )
        .expect("create capture schema");
}

fn create_slot(connection: &mut PgConnection, name: &str) {
    // Logical replication slot administration has no Diesel query DSL form.
    diesel::sql_query("SELECT pg_create_logical_replication_slot($1, 'wal2json')")
        .bind::<diesel::sql_types::Text, _>(name)
        .execute(connection)
        .expect("create wal2json slot");
}

fn write_changes(connection: &mut PgConnection) {
    let amount =
        BigDecimal::from_str("12345678901234567890.123456789").expect("valid numeric fixture");
    let observed_at = Utc
        .with_ymd_and_hms(2026, 8, 28, 7, 4, 56)
        .single()
        .expect("valid fixture timestamp");
    let toasted_text = "x".repeat(10_000);

    connection
        .transaction::<_, diesel::result::Error, _>(|connection| {
            diesel::insert_into(default_rows::table)
                .values((
                    default_rows::tenant_id.eq(7),
                    default_rows::id.eq(42),
                    default_rows::payload.eq(vec![0xde, 0xad, 0xbe, 0xef]),
                    default_rows::amount.eq(amount),
                    default_rows::observed_at.eq(observed_at),
                    default_rows::nullable_text.eq(None::<String>),
                    default_rows::toasted_text.eq(toasted_text),
                    default_rows::status.eq("inserted"),
                ))
                .execute(connection)?;

            diesel::update(default_rows::table.find((7, 42)))
                .set(default_rows::status.eq("updated"))
                .execute(connection)?;

            diesel::delete(default_rows::table.find((7, 42))).execute(connection)?;

            diesel::insert_into(full_rows::table)
                .values((
                    full_rows::id.eq(9),
                    full_rows::code.eq("full"),
                    full_rows::payload.eq("before"),
                ))
                .execute(connection)?;

            diesel::update(full_rows::table.find(9))
                .set(full_rows::payload.eq("after"))
                .execute(connection)?;

            diesel::delete(full_rows::table.find(9)).execute(connection)?;

            diesel::insert_into(truncate_rows::table)
                .values(truncate_rows::id.eq(1))
                .execute(connection)?;

            // Diesel has no typed TRUNCATE statement.
            diesel::sql_query("TRUNCATE truncate_rows").execute(connection)?;

            Ok(())
        })
        .expect("write capture transaction");
}

fn write_all_options_changes(connection: &mut PgConnection) {
    let amount =
        BigDecimal::from_str("12345678901234567890.123456789").expect("valid numeric fixture");

    // A non-transactional message becomes its own wal2json object. No DSL form for this function.
    diesel::sql_query("SELECT pg_logical_emit_message(false, $1, 'non-transactional')")
        .bind::<diesel::sql_types::Text, _>(MESSAGE_PREFIX)
        .execute(connection)
        .expect("emit non-transactional message");

    connection
        .transaction::<_, diesel::result::Error, _>(|connection| {
            diesel::insert_into(all_options_rows::table)
                .values((
                    all_options_rows::id.eq(1),
                    all_options_rows::label.eq("first"),
                    all_options_rows::amount.eq(Some(amount)),
                    all_options_rows::note.eq(None::<String>),
                ))
                .execute(connection)?;

            diesel::update(all_options_rows::table.find(1))
                .set(all_options_rows::note.eq("second"))
                .execute(connection)?;

            diesel::delete(all_options_rows::table.find(1)).execute(connection)?;

            // Only the id, so the DEFAULT on label is what fills it in.
            diesel::insert_into(all_options_rows::table)
                .values(all_options_rows::id.eq(2))
                .execute(connection)?;

            // Diesel has no typed TRUNCATE statement.
            diesel::sql_query("TRUNCATE all_options_rows").execute(connection)?;

            // No DSL form.
            diesel::sql_query("SELECT pg_logical_emit_message(true, $1, 'transactional')")
                .bind::<diesel::sql_types::Text, _>(MESSAGE_PREFIX)
                .execute(connection)?;

            Ok(())
        })
        .expect("write all-options capture transaction");
}

fn drain_slot(connection: &mut PgConnection, slot: &str, format_version: &str) -> Vec<String> {
    // Plugin options are variadic arguments, so no DSL form.
    diesel::sql_query(
        "SELECT data FROM pg_logical_slot_get_changes(
             $1, NULL, NULL,
             'format-version', $2,
             'include-transaction', 'false',
             'actions', 'insert,update,delete,truncate'
         )",
    )
    .bind::<diesel::sql_types::Text, _>(slot)
    .bind::<diesel::sql_types::Text, _>(format_version)
    .load::<CapturedRow>(connection)
    .expect("drain wal2json slot")
    .into_iter()
    .map(|row| row.data)
    .collect()
}

/// Drains with every field-adding option. Repeats until empty: the non-transactional message and
/// the transaction need not arrive in one batch.
fn drain_slot_all_options(
    connection: &mut PgConnection,
    slot: &str,
    format_version: &str,
) -> Vec<String> {
    let mut lines = Vec::new();

    for _ in 0..4 {
        // Plugin options are variadic arguments, so no DSL form.
        let batch: Vec<String> = diesel::sql_query(
            "SELECT data FROM pg_logical_slot_get_changes(
                 $1, NULL, NULL,
                 'format-version', $2,
                 'actions', 'insert,update,delete,truncate',
                 'include-xids', 'true',
                 'include-timestamp', 'true',
                 'include-origin', 'true',
                 'include-lsn', 'true',
                 'include-pk', 'true',
                 'include-type-oids', 'true',
                 'include-not-null', 'true',
                 'include-default', 'true',
                 'include-column-positions', 'true'
             )",
        )
        .bind::<diesel::sql_types::Text, _>(slot)
        .bind::<diesel::sql_types::Text, _>(format_version)
        .load::<CapturedRow>(connection)
        .expect("drain wal2json slot")
        .into_iter()
        .map(|row| row.data)
        .collect();

        if batch.is_empty() {
            break;
        }
        lines.extend(batch);
    }

    lines
}

// The values a capture cannot reproduce. Both normalizers below use these, so a written fixture is
// already normalized and regenerating twice gives the same bytes.
const XID: u32 = 1000;
const TIMESTAMP: &str = "2026-08-28 00:00:00+00";
const LSN: &str = "0/1000000";

/// Replaces transaction ids, timestamps and LSNs. An explicit null stays: wal2json means "does not
/// apply" by it.
fn normalize_volatile(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, field) in map.iter_mut() {
                if field.is_null() {
                    continue;
                }
                match key.as_str() {
                    "xid" => *field = json!(XID),
                    "timestamp" => *field = json!(TIMESTAMP),
                    "lsn" | "nextlsn" => *field = json!(LSN),
                    _ => normalize_volatile(field),
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                normalize_volatile(item);
            }
        }
        _ => {}
    }
}

/// The same, editing the four scalars in place rather than re-serializing, so key order and the
/// digits of a `numeric` literal survive.
fn with_placeholders(line: &str) -> String {
    let replaced = replace_scalar(line, "xid", &XID.to_string());
    let replaced = replace_scalar(&replaced, "timestamp", &format!("\"{TIMESTAMP}\""));
    let replaced = replace_scalar(&replaced, "lsn", &format!("\"{LSN}\""));
    let replaced = replace_scalar(&replaced, "nextlsn", &format!("\"{LSN}\""));

    // Proves the textual edit touched nothing else, so it cannot go wrong silently.
    assert_eq!(
        normalized([line]),
        normalized([replaced.as_str()]),
        "placeholder substitution changed something other than the volatile values"
    );

    replaced
}

/// Replaces the scalar after every `"key":`, leaving an explicit null alone. These four are a bare
/// number or an escape-free string, so finding the end needs no JSON parsing.
fn replace_scalar(line: &str, key: &str, replacement: &str) -> String {
    let needle = format!("\"{key}\":");
    let mut out = String::with_capacity(line.len());
    let mut rest = line;

    while let Some(at) = rest.find(&needle) {
        let (before, after) = rest.split_at(at + needle.len());
        out.push_str(before);

        let end = if let Some(inside) = after.strip_prefix('"') {
            inside.find('"').expect("closing quote") + 2
        } else {
            after.find([',', '}']).expect("end of value")
        };

        if &after[..end] == "null" {
            out.push_str("null");
        } else {
            out.push_str(replacement);
        }
        rest = &after[end..];
    }

    out.push_str(rest);
    out
}

fn normalized<'a>(lines: impl IntoIterator<Item = &'a str>) -> Vec<Value> {
    lines
        .into_iter()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let mut value: Value = serde_json::from_str(line).expect("captured line is JSON");
            normalize_volatile(&mut value);
            value
        })
        .collect()
}

/// The four corpora, captured from one live server session.
struct Captured {
    v1: Vec<String>,
    v2: Vec<String>,
    v1_all: Vec<String>,
    v2_all: Vec<String>,
}

impl Captured {
    /// What each fixture file should contain. Per-run identifiers are replaced, so an unchanged
    /// database writes byte-identical files.
    fn files(&self) -> [(&'static str, String); 4] {
        fn contents(lines: &[String]) -> String {
            let replaced: Vec<String> = lines.iter().map(|line| with_placeholders(line)).collect();
            format!("{}\n", replaced.join("\n"))
        }

        [
            (V1_PATH, contents(&self.v1)),
            (V2_PATH, contents(&self.v2)),
            (V1_ALL_PATH, contents(&self.v1_all)),
            (V2_ALL_PATH, contents(&self.v2_all)),
        ]
    }
}

fn capture() -> Captured {
    let container = postgres();
    let mut connection = connect(&container);
    create_schema(&mut connection);
    create_slot(&mut connection, "capture_v1");
    create_slot(&mut connection, "capture_v2");
    write_changes(&mut connection);

    let v1 = drain_slot(&mut connection, "capture_v1", "1");
    let v2 = drain_slot(&mut connection, "capture_v2", "2");

    assert_eq!(v1.len(), 1, "expected one v1 transaction");
    assert_eq!(v2.len(), 8, "expected eight v2 changes");

    // Created after the first drain, so they carry only the second transaction.
    create_slot(&mut connection, "capture_v1_all");
    create_slot(&mut connection, "capture_v2_all");
    write_all_options_changes(&mut connection);

    let v1_all = drain_slot_all_options(&mut connection, "capture_v1_all", "1");
    let v2_all = drain_slot_all_options(&mut connection, "capture_v2_all", "2");

    assert!(!v1_all.is_empty(), "expected v1 all-options output");
    assert!(!v2_all.is_empty(), "expected v2 all-options output");

    // The part a committed fixture cannot prove about itself.
    parse_v1(&v1[0]).expect("parse live v1");
    for line in &v2 {
        parse_v2(line).expect("parse live v2 line");
    }
    for line in &v1_all {
        parse_v1(line).expect("parse live v1 all-options line");
    }
    for line in &v2_all {
        parse_v2(line).expect("parse live v2 all-options line");
    }

    Captured {
        v1,
        v2,
        v1_all,
        v2_all,
    }
}

/// Checks the committed fixtures against a live server. Writes nothing, so a drift stays visible.
#[test]
#[ignore = "captures from PostgreSQL using Docker"]
fn capture_matches_committed_fixtures() {
    for (path, captured) in capture().files() {
        let committed =
            fs::read_to_string(path).unwrap_or_else(|error| panic!("read {path}: {error}"));

        assert_eq!(
            normalized(captured.lines()),
            normalized(committed.lines()),
            "{path} no longer matches what the server produces"
        );
    }
}

/// Rewrites the fixtures from a live server. Run when the corpus should change, then read the diff.
#[test]
#[ignore = "rewrites the committed fixtures using Docker"]
fn regenerate_committed_fixtures() {
    for (path, captured) in capture().files() {
        fs::write(path, captured).unwrap_or_else(|error| panic!("write {path}: {error}"));
    }
}
