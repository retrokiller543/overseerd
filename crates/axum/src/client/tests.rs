use super::{encode_path_segment, encode_path_segments};

#[test]
fn single_path_segment_encodes_separators() {
    assert_eq!(encode_path_segment("docs/api x.md"), "docs%2Fapi%20x.md");
}

#[test]
fn catch_all_path_preserves_separators_and_encodes_each_segment() {
    assert_eq!(
        encode_path_segments("docs/api guide/x#1.md"),
        "docs/api%20guide/x%231.md"
    );
}

#[test]
fn catch_all_path_preserves_empty_segments() {
    assert_eq!(encode_path_segments("/docs//index/"), "/docs//index/");
}
