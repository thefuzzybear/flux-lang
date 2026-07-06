use crate::error::CompileErrorWithSpan;

/// Get the content of a specific line (1-based) from source.
/// Returns an empty string if the line number is out of range.
fn get_source_line(source: &str, line_num: usize) -> &str {
    source.lines().nth(line_num - 1).unwrap_or("")
}

/// Format an error with optional ANSI coloring and source context.
///
/// Renders the error header (`error[file:line:col]: message`) with optional color,
/// followed by the source line containing the error and a caret (`^`) pointing
/// to the error column.
///
/// When `use_color` is true:
/// - `error` is rendered in bold red (`\x1b[1;31m`)
/// - `file:line:col` is rendered in cyan (`\x1b[36m`)
///
/// Edge case: if the offset points to end-of-file, the last source line is shown
/// with the caret positioned one column past the last character.
pub fn format_error_colored(
    file: &str,
    source: &str,
    offset: usize,
    message: &str,
    use_color: bool,
) -> String {
    let (line_num, col) = byte_offset_to_line_col(source, offset);

    // Build header
    let header = if use_color {
        format!(
            "\x1b[1;31merror\x1b[0m[\x1b[36m{}:{}:{}\x1b[0m]: {}",
            file, line_num, col, message
        )
    } else {
        format!("error[{}:{}:{}]: {}", file, line_num, col, message)
    };

    // Get the source line
    let source_line = get_source_line(source, line_num);

    // Build the line number gutter width
    let line_num_str = line_num.to_string();
    let gutter_width = line_num_str.len();

    // Source line: "   3 | x = if y"
    let source_display = format!(
        "{:>width$} | {}",
        line_num,
        source_line,
        width = gutter_width
    );

    // Caret line: the caret goes at column position (col - 1) offset by the gutter
    // Gutter takes: gutter_width + " | " (3 chars)
    let gutter_padding = " ".repeat(gutter_width + 3); // digits + " | "
    let col_padding = " ".repeat(col.saturating_sub(1));
    let caret_line = format!("{}{}^", gutter_padding, col_padding);

    format!("{}\n{}\n{}", header, source_display, caret_line)
}

/// Converts a byte offset in a source string to a 1-based (line, column) pair.
///
/// - Lines are 1-based (first line is line 1).
/// - Columns count bytes from the start of the line, 1-based.
/// - If `offset` exceeds the source length, it is clamped to `source.len()`.
/// - A newline character at the offset is considered part of the current line
///   (we count newlines strictly *before* the offset).
pub fn byte_offset_to_line_col(source: &str, offset: usize) -> (usize, usize) {
    // Clamp offset to source length
    let offset = offset.min(source.len());

    let bytes = source.as_bytes();

    let mut line = 1;
    let mut last_newline_pos: Option<usize> = None;

    for i in 0..offset {
        if bytes[i] == b'\n' {
            line += 1;
            last_newline_pos = Some(i);
        }
    }

    let col = match last_newline_pos {
        Some(pos) => offset - pos,
        None => offset + 1,
    };

    (line, col)
}

/// Formats a single compile error with file location.
///
/// Output format: `error[{file}:{line}:{col}]: {message}`
pub fn format_error(file: &str, source: &str, offset: usize, message: &str) -> String {
    let (line, col) = byte_offset_to_line_col(source, offset);
    format!("error[{file}:{line}:{col}]: {message}")
}

/// Formats multiple compile errors, sorted by byte offset ascending.
///
/// Each error is formatted on its own line using `format_error`.
/// The input slice is not mutated; a sorted copy is created internally.
pub fn format_errors(file: &str, source: &str, errors: &[CompileErrorWithSpan]) -> String {
    let mut sorted: Vec<&CompileErrorWithSpan> = errors.iter().collect();
    sorted.sort_by_key(|e| e.offset);
    sorted
        .iter()
        .map(|e| format_error(file, source, e.offset, &e.message))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn offset_zero_returns_1_1() {
        assert_eq!(byte_offset_to_line_col("hello", 0), (1, 1));
    }

    #[test]
    fn offset_zero_empty_source() {
        assert_eq!(byte_offset_to_line_col("", 0), (1, 1));
    }

    #[test]
    fn first_line_middle() {
        // "hello" offset 3 → line 1, col 4
        assert_eq!(byte_offset_to_line_col("hello", 3), (1, 4));
    }

    #[test]
    fn at_newline_char() {
        // "ab\ncd" — offset 2 is the '\n' itself, which is on line 1
        // newlines before offset 2: none (the '\n' is AT offset 2, not before it)
        assert_eq!(byte_offset_to_line_col("ab\ncd", 2), (1, 3));
    }

    #[test]
    fn start_of_second_line() {
        // "ab\ncd" — offset 3 is 'c', first char of line 2
        // newlines before offset 3: one at position 2
        // col = 3 - 2 = 1
        assert_eq!(byte_offset_to_line_col("ab\ncd", 3), (2, 1));
    }

    #[test]
    fn multiple_lines() {
        // "a\nb\nc" — offsets: a=0, \n=1, b=2, \n=3, c=4
        assert_eq!(byte_offset_to_line_col("a\nb\nc", 0), (1, 1)); // 'a'
        assert_eq!(byte_offset_to_line_col("a\nb\nc", 1), (1, 2)); // '\n' on line 1
        assert_eq!(byte_offset_to_line_col("a\nb\nc", 2), (2, 1)); // 'b'
        assert_eq!(byte_offset_to_line_col("a\nb\nc", 3), (2, 2)); // '\n' on line 2
        assert_eq!(byte_offset_to_line_col("a\nb\nc", 4), (3, 1)); // 'c'
    }

    #[test]
    fn offset_past_end_clamped() {
        // "hi" has len 2; offset 100 should clamp to offset 2
        // No newlines, col = 2 + 1 = 3? No: clamped offset = 2, no newlines before → col = 2+1 = 3
        // Actually line 1, col 3 (one past the last char)
        assert_eq!(byte_offset_to_line_col("hi", 100), (1, 3));
    }

    #[test]
    fn offset_past_end_with_newlines() {
        // "a\nb" has len 3; offset 999 clamps to 3
        // newlines before offset 3: one at position 1
        // col = 3 - 1 = 2
        assert_eq!(byte_offset_to_line_col("a\nb", 999), (2, 2));
    }

    #[test]
    fn consecutive_newlines() {
        // "\n\n" — offset 0 is first '\n' (line 1, col 1)
        // offset 1 is second '\n' — newlines before: one at 0 → line 2, col = 1 - 0 = 1
        assert_eq!(byte_offset_to_line_col("\n\n", 0), (1, 1));
        assert_eq!(byte_offset_to_line_col("\n\n", 1), (2, 1));
    }

    #[test]
    fn source_starts_with_newline() {
        // "\nhello" — offset 0 is '\n' (line 1, col 1)
        // offset 1 is 'h' — newlines before: one at 0 → line 2, col = 1 - 0 = 1
        assert_eq!(byte_offset_to_line_col("\nhello", 0), (1, 1));
        assert_eq!(byte_offset_to_line_col("\nhello", 1), (2, 1));
        assert_eq!(byte_offset_to_line_col("\nhello", 3), (2, 3));
    }

    // **Validates: Requirements 6.1, 6.2**
    //
    // Property 9: Byte offset to line:column correctness
    // For any source string containing at least one character and any valid byte offset
    // within that string, the computed (line, col) shall satisfy:
    // - line equals the number of '\n' characters before the offset plus 1
    // - col equals offset minus byte position of last '\n' before offset (or offset + 1 if none)
    proptest! {
        #[test]
        fn prop_byte_offset_to_line_col_correctness(
            source in "[a-z\\n ]{1,200}",
            index in 0usize..200,
        ) {
            // Ensure we have a non-empty source and a valid offset
            prop_assume!(!source.is_empty());
            let offset = index % source.len();

            let (line, col) = byte_offset_to_line_col(&source, offset);

            // Independently compute expected values
            let prefix = &source[..offset];
            let expected_line = prefix.chars().filter(|&c| c == '\n').count() + 1;
            let expected_col = match prefix.rfind('\n') {
                Some(pos) => offset - pos,
                None => offset + 1,
            };

            prop_assert_eq!(
                (line, col),
                (expected_line, expected_col),
                "source={:?}, offset={}", source, offset
            );
        }
    }

    // --- Tests for format_error ---

    #[test]
    fn format_error_first_line() {
        let source = "let x = 42\nlet y = 10";
        let result = format_error("test.flux", source, 4, "unexpected token");
        assert_eq!(result, "error[test.flux:1:5]: unexpected token");
    }

    #[test]
    fn format_error_second_line() {
        // "let x = 42\n" is 11 bytes, so offset 11 is start of line 2
        let source = "let x = 42\nlet y = 10";
        let result = format_error("test.flux", source, 11, "type mismatch");
        assert_eq!(result, "error[test.flux:2:1]: type mismatch");
    }

    #[test]
    fn format_error_offset_zero() {
        let source = "hello world";
        let result = format_error("main.flux", source, 0, "parse error");
        assert_eq!(result, "error[main.flux:1:1]: parse error");
    }

    // --- Tests for format_errors ---

    #[test]
    fn format_errors_sorts_by_offset() {
        let source = "aaa\nbbb\nccc";
        let errors = vec![
            CompileErrorWithSpan { offset: 8, message: "third".to_string() },
            CompileErrorWithSpan { offset: 0, message: "first".to_string() },
            CompileErrorWithSpan { offset: 4, message: "second".to_string() },
        ];
        let result = format_errors("test.flux", source, &errors);
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "error[test.flux:1:1]: first");
        assert_eq!(lines[1], "error[test.flux:2:1]: second");
        assert_eq!(lines[2], "error[test.flux:3:1]: third");
    }

    #[test]
    fn format_errors_empty_vec() {
        let source = "hello";
        let errors: Vec<CompileErrorWithSpan> = vec![];
        let result = format_errors("test.flux", source, &errors);
        assert_eq!(result, "");
    }

    #[test]
    fn format_errors_single_error() {
        let source = "let x = 1";
        let errors = vec![
            CompileErrorWithSpan { offset: 6, message: "bad value".to_string() },
        ];
        let result = format_errors("test.flux", source, &errors);
        assert_eq!(result, "error[test.flux:1:7]: bad value");
    }

    // --- Tests for format_error_colored ---

    #[test]
    fn format_error_colored_with_color() {
        let source = "x = if y\nz = 10";
        // offset 4 is 'i' in 'if' → line 1, col 5
        let result = format_error_colored("file.flux", source, 4, "unexpected token 'if'", true);
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines[0],
            "\x1b[1;31merror\x1b[0m[\x1b[36mfile.flux:1:5\x1b[0m]: unexpected token 'if'"
        );
        assert_eq!(lines[1], "1 | x = if y");
        assert_eq!(lines[2], "        ^");
    }

    #[test]
    fn format_error_colored_without_color() {
        let source = "x = if y\nz = 10";
        let result = format_error_colored("file.flux", source, 4, "unexpected token 'if'", false);
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "error[file.flux:1:5]: unexpected token 'if'");
        assert_eq!(lines[1], "1 | x = if y");
        assert_eq!(lines[2], "        ^");
    }

    #[test]
    fn format_error_colored_second_line() {
        let source = "let x = 42\nlet y = bad";
        // offset 15 is 'b' in 'bad' → line 2, col 9-offset from newline at pos 10: 15-10 = 5
        // Actually: newline at pos 10, so col = 15 - 10 = 5
        let result = format_error_colored("test.flux", source, 15, "unknown identifier", false);
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines[0], "error[test.flux:2:5]: unknown identifier");
        assert_eq!(lines[1], "2 | let y = bad");
        assert_eq!(lines[2], "        ^");
    }

    #[test]
    fn format_error_colored_multi_digit_line_number() {
        // Create source with 10+ lines
        let source = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl";
        // offset for 'k' on line 11: each line is 2 bytes (char + newline) except last
        // 'k' is at offset 20 → line 11, col 1
        let result = format_error_colored("test.flux", source, 20, "error here", false);
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines[0], "error[test.flux:11:1]: error here");
        assert_eq!(lines[1], "11 | k");
        // gutter is "11 | " (5 chars), col 1 means caret under 'k' → 5 spaces + ^
        assert_eq!(lines[2], "     ^");
    }

    #[test]
    fn format_error_colored_offset_at_eof() {
        let source = "hello";
        // offset 5 == source.len(), clamped → line 1, col 6 (one past last char)
        let result = format_error_colored("test.flux", source, 5, "unexpected EOF", false);
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines[0], "error[test.flux:1:6]: unexpected EOF");
        assert_eq!(lines[1], "1 | hello");
        assert_eq!(lines[2], "         ^");
    }

    #[test]
    fn format_error_colored_empty_source() {
        let source = "";
        // offset 0 on empty → line 1, col 1
        let result = format_error_colored("test.flux", source, 0, "empty file", false);
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines[0], "error[test.flux:1:1]: empty file");
        assert_eq!(lines[1], "1 | ");
        assert_eq!(lines[2], "    ^");
    }

    #[test]
    fn format_error_colored_offset_at_first_char() {
        let source = "hello world";
        // offset 0 → line 1, col 1
        let result = format_error_colored("test.flux", source, 0, "bad start", false);
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines[0], "error[test.flux:1:1]: bad start");
        assert_eq!(lines[1], "1 | hello world");
        assert_eq!(lines[2], "    ^");
    }

    // **Validates: Requirements 2.9, 6.3**
    //
    // Property 1: Error ordering by byte offset
    // For any set of CompileErrorWithSpan entries with distinct byte offsets,
    // formatting them for display shall produce lines ordered in strictly
    // ascending byte offset order.
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_error_ordering_by_byte_offset(
            source in "[a-z\\n ]{100,500}",
            raw_offsets in prop::collection::vec(0usize..500, 1..20),
            messages in prop::collection::vec("[a-z ]{1,30}", 1..20),
        ) {
            let source_len = source.len();
            prop_assume!(source_len > 0);

            // Generate distinct offsets within valid range
            let mut offsets: Vec<usize> = raw_offsets
                .iter()
                .map(|&o| o % source_len)
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();

            prop_assume!(!offsets.is_empty());

            // Build errors with shuffled (arbitrary HashSet) order to ensure
            // format_errors actually sorts them
            let errors: Vec<CompileErrorWithSpan> = offsets
                .iter()
                .zip(messages.iter().cycle())
                .map(|(&offset, msg)| CompileErrorWithSpan {
                    offset,
                    message: msg.clone(),
                })
                .collect();

            let output = format_errors("test.flux", &source, &errors);

            // Parse line:col from each output line
            // Format: error[test.flux:{line}:{col}]: {message}
            let mut extracted_line_cols: Vec<(usize, usize)> = Vec::new();
            for line in output.lines() {
                let after_file = line
                    .strip_prefix("error[test.flux:")
                    .expect("line should start with error[test.flux:");
                let coords_end = after_file.find(']').expect("line should contain ]");
                let coords = &after_file[..coords_end];
                let parts: Vec<&str> = coords.split(':').collect();
                prop_assert_eq!(parts.len(), 2, "Expected line:col format, got {:?}", coords);
                let l: usize = parts[0].parse().expect("line number should parse");
                let c: usize = parts[1].parse().expect("col number should parse");
                extracted_line_cols.push((l, c));
            }

            // Verify output line count matches error count
            prop_assert_eq!(
                extracted_line_cols.len(),
                errors.len(),
                "Expected {} error lines, got {}",
                errors.len(),
                extracted_line_cols.len()
            );

            // Verify (line, col) pairs are in strictly ascending order
            // Since format_errors sorts by distinct offsets and byte_offset_to_line_col
            // is strictly monotonic for distinct offsets, tuples must be strictly increasing.
            for i in 1..extracted_line_cols.len() {
                prop_assert!(
                    extracted_line_cols[i] > extracted_line_cols[i - 1],
                    "Error lines not in strictly ascending (line, col) order: {:?} is not > {:?} at index {}",
                    extracted_line_cols[i],
                    extracted_line_cols[i - 1],
                    i
                );
            }

            // Cross-check: sorted offsets should produce the same (line, col) sequence
            offsets.sort();
            for (i, &expected_offset) in offsets.iter().enumerate() {
                let expected_lc = byte_offset_to_line_col(&source, expected_offset);
                prop_assert_eq!(
                    extracted_line_cols[i],
                    expected_lc,
                    "Mismatch at error index {}: offset={}", i, expected_offset
                );
            }
        }
    }
}
