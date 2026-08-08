use super::write_terminal_safe;

#[test]
fn component_output_without_color_escapes_terminal_controls() {
    let mut output = Vec::new();

    write_terminal_safe(b"plain\x1b[31m\0\r\n", false, &mut output).expect("output writes");

    assert_eq!(output, b"plain\\x1b[31m\\x00\\x0d\n");
}

#[test]
fn color_output_allows_only_sgr_escape_sequences() {
    let mut output = Vec::new();

    write_terminal_safe(
        b"\x1b[31mred\x1b[0m\x1b]52;clipboard\x07",
        true,
        &mut output,
    )
    .expect("output writes");

    assert_eq!(output, b"\x1b[31mred\x1b[0m\\x1b]52;clipboard\\x07");
}

#[test]
fn terminal_output_escapes_bidirectional_formatting() {
    let mut output = Vec::new();

    write_terminal_safe("safe\u{202e}txt".as_bytes(), false, &mut output).expect("output writes");

    assert_eq!(output, b"safe\\u{202e}txt");
}

#[test]
fn terminal_output_escapes_utf8_c1_controls() {
    let mut output = Vec::new();

    write_terminal_safe("safe\u{009b}31m".as_bytes(), true, &mut output).expect("output writes");

    assert_eq!(output, b"safe\\u{9b}31m");
}

#[test]
fn overlong_sgr_sequence_is_escaped() {
    let mut output = Vec::new();
    let rendered = format!("\x1b[{}m", "1;".repeat(20));

    write_terminal_safe(rendered.as_bytes(), true, &mut output).expect("output writes");

    assert!(output.starts_with(b"\\x1b["));
}
