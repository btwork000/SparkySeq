//! Show a pattern file next to its re-encoded form, record by record: `ptn_dump FILE`.
use sp404_formats::pattern_edit::EditablePattern;

fn main() {
    let path = std::env::args().nth(1).expect("usage: ptn_dump FILE");
    let bytes = std::fs::read(&path).unwrap();
    let enc = EditablePattern::parse(&bytes).unwrap().encode();
    let n = bytes.len().max(enc.len()) / 8;
    for i in 0..n {
        let show = |b: &[u8]| {
            b.get(i * 8..i * 8 + 8)
                .map(|r| format!("{r:02x?}"))
                .unwrap_or_default()
        };
        println!("{:42} {}", show(&bytes), show(&enc));
    }
}
