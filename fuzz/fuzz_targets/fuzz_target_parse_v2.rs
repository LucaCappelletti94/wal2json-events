#![no_main]
use libfuzzer_sys::fuzz_target;
use serde_json::Value;
use wal2json_events_fuzz::assert_nothing_invented;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(parsed) = wal2json_events::parse_v2(text) else {
        return;
    };

    let serialized = serde_json::to_string(&parsed).expect("serialization should not fail");
    let reparsed = wal2json_events::parse_v2(&serialized)
        .expect("reparse should not fail after serialization");
    assert_eq!(parsed, reparsed, "parsed and reparsed should be equal");

    // The model accepts input a Value cannot hold: skipping an unknown field never requires
    // representing its number. The comparison needs both representations.
    let Ok(input) = serde_json::from_str::<Value>(text) else {
        return;
    };
    let output = serde_json::to_value(&parsed).expect("serializing a parsed message cannot fail");
    assert_nothing_invented(&input, &output, "");
});
