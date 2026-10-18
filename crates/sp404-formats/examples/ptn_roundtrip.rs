//! Check that re-encoding pattern files keeps their content: `ptn_roundtrip FILE...`.
use sp404_formats::Pattern;
use sp404_formats::pattern_edit::EditablePattern;

fn main() {
    let mut failed = false;
    for path in std::env::args().skip(1) {
        let bytes = std::fs::read(&path).unwrap();
        let edit = EditablePattern::parse(&bytes).unwrap();
        let encoded = edit.encode();
        let same_events = Pattern::parse(&encoded).unwrap() == Pattern::parse(&bytes).unwrap();
        let same_records = EditablePattern::parse(&encoded).unwrap() == edit;
        let identical = encoded == bytes;
        println!(
            "{path}: events {same_events}, records {same_records}, byte-identical {identical}"
        );
        failed |= !(same_events && same_records);
    }
    std::process::exit(failed as i32);
}
