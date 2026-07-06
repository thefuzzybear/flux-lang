//! Property test for error caret positioning.
//!
//! Feature: flux-fmt-syntax-highlight, Property 10: Error Caret Positioning
//!
//! **Validates: Requirements 9.3, 9.6**
//!
//! For any source string and any valid byte offset within that string, the error
//! diagnostic renderer produces a caret line where the `^` character is positioned
//! at exactly the correct column corresponding to the byte offset within its
//! source line.

use proptest::prelude::*;

use flux_cli::diagnostics::{byte_offset_to_line_col, format_error_colored};

// =============================================================================
// Property 10: Error Caret Positioning
// =============================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// **Validates: Requirements 9.3, 9.6**
    ///
    /// For any source string and any valid byte offset, the caret `^` in the
    /// rendered diagnostic is positioned at the correct column:
    ///   gutter_width + 3 (for " | ") + (col - 1)
    ///
    /// where col is the 1-based column from byte_offset_to_line_col.
    #[test]
    fn prop_caret_at_correct_column(
        source in "[a-z0-9 \\n]{1,300}",
        raw_offset in 0usize..300,
    ) {
        prop_assume!(!source.is_empty());
        let offset = raw_offset % (source.len() + 1); // allow offset == source.len() (EOF)

        let output = format_error_colored("test.flux", &source, offset, "test error", false);
        let lines: Vec<&str> = output.lines().collect();

        // Output should have exactly 3 lines: header, source display, caret
        prop_assert_eq!(
            lines.len(), 3,
            "Expected 3 output lines, got {}: {:?}", lines.len(), lines
        );

        let caret_line = lines[2];

        // The caret line should contain exactly one '^'
        let caret_count = caret_line.chars().filter(|&c| c == '^').count();
        prop_assert_eq!(
            caret_count, 1,
            "Expected exactly one '^' in caret line, got {}: {:?}", caret_count, caret_line
        );

        // Find the position of '^' in the caret line
        let caret_pos = caret_line.find('^').unwrap();

        // Compute expected position using the same logic as the implementation:
        // gutter_width (digits of line number) + 3 (" | ") + (col - 1)
        let (line_num, col) = byte_offset_to_line_col(&source, offset);
        let gutter_width = line_num.to_string().len();
        let expected_caret_pos = gutter_width + 3 + (col - 1);

        prop_assert_eq!(
            caret_pos, expected_caret_pos,
            "Caret position mismatch for source={:?}, offset={}, line={}, col={}: \
             expected caret at {}, got {} in line {:?}",
            &source[..source.len().min(50)], offset, line_num, col,
            expected_caret_pos, caret_pos, caret_line
        );
    }

    /// **Validates: Requirements 9.3, 9.6**
    ///
    /// The column number reported in the error header matches the caret position
    /// in the caret line (both point to the same column).
    #[test]
    fn prop_header_col_matches_caret_position(
        source in "[a-z0-9 \\n]{1,200}",
        raw_offset in 0usize..200,
    ) {
        prop_assume!(!source.is_empty());
        let offset = raw_offset % (source.len() + 1);

        let output = format_error_colored("test.flux", &source, offset, "test", false);
        let lines: Vec<&str> = output.lines().collect();
        prop_assert_eq!(lines.len(), 3);

        // Parse line:col from header: "error[test.flux:{line}:{col}]: test"
        let header = lines[0];
        let after_prefix = header.strip_prefix("error[test.flux:").unwrap();
        let coords_end = after_prefix.find(']').unwrap();
        let coords = &after_prefix[..coords_end];
        let parts: Vec<&str> = coords.split(':').collect();
        prop_assert_eq!(parts.len(), 2);
        let header_line: usize = parts[0].parse().unwrap();
        let header_col: usize = parts[1].parse().unwrap();

        // Verify the source display line shows the correct line number
        let source_display = lines[1];
        let trimmed = source_display.trim_start();
        let line_num_end = trimmed.find(' ').unwrap_or(trimmed.len());
        let displayed_line_num: usize = trimmed[..line_num_end].parse().unwrap_or(0);
        prop_assert_eq!(
            displayed_line_num, header_line,
            "Source line number in gutter doesn't match header"
        );

        // Verify caret position corresponds to header_col
        let caret_line = lines[2];
        let caret_pos = caret_line.find('^').unwrap();
        let gutter_width = header_line.to_string().len();
        let expected_caret_pos = gutter_width + 3 + (header_col - 1);

        prop_assert_eq!(
            caret_pos, expected_caret_pos,
            "Header reports col={} but caret at position {} (expected {})",
            header_col, caret_pos, expected_caret_pos
        );
    }

    /// **Validates: Requirements 9.3, 9.6**
    ///
    /// Edge case: offset at position 0 always produces a caret at the first
    /// column (right after the gutter).
    #[test]
    fn prop_offset_zero_caret_at_first_col(
        source in "[a-z0-9 ]{1,100}",
    ) {
        // Source with no newlines: offset 0 → line 1, col 1
        prop_assume!(!source.is_empty());
        prop_assume!(!source.contains('\n'));

        let output = format_error_colored("test.flux", &source, 0, "test", false);
        let lines: Vec<&str> = output.lines().collect();
        prop_assert_eq!(lines.len(), 3);

        let caret_line = lines[2];
        let caret_pos = caret_line.find('^').unwrap();

        // Line 1 → gutter_width = 1, so caret at position 1 + 3 + 0 = 4
        prop_assert_eq!(
            caret_pos, 4,
            "Offset 0 caret should be at position 4 (gutter '1 | '), got {}",
            caret_pos
        );
    }

    /// **Validates: Requirements 9.3, 9.6**
    ///
    /// Edge case: offset at end of source (EOF) positions the caret one past
    /// the last character of the last line.
    #[test]
    fn prop_offset_at_eof_caret_past_last_char(
        source in "[a-z0-9]{1,50}",
    ) {
        // Source with no newlines to simplify: offset == len → col = len + 1
        prop_assume!(!source.is_empty());
        prop_assume!(!source.contains('\n'));

        let offset = source.len(); // EOF position

        let output = format_error_colored("test.flux", &source, offset, "eof", false);
        let lines: Vec<&str> = output.lines().collect();
        prop_assert_eq!(lines.len(), 3);

        let caret_line = lines[2];
        let caret_pos = caret_line.find('^').unwrap();

        // line 1, col = source.len() + 1
        // gutter_width = 1, expected position = 1 + 3 + source.len()
        let expected = 1 + 3 + source.len();
        prop_assert_eq!(
            caret_pos, expected,
            "EOF caret should be at {} (past last char), got {}", expected, caret_pos
        );
    }

    /// **Validates: Requirements 9.3, 9.6**
    ///
    /// For multi-line sources, the caret still correctly points to the column
    /// within the specific line where the offset falls.
    #[test]
    fn prop_multiline_caret_correct(
        lines_content in proptest::collection::vec("[a-z0-9]{1,20}", 2..10),
    ) {
        let source = lines_content.join("\n");
        prop_assume!(!source.is_empty());

        // Pick an offset somewhere in the middle of the source
        let mid_offset = source.len() / 2;

        let output = format_error_colored("test.flux", &source, mid_offset, "mid", false);
        let output_lines: Vec<&str> = output.lines().collect();
        prop_assert_eq!(output_lines.len(), 3);

        // Verify the caret position matches our independent computation
        let (line_num, col) = byte_offset_to_line_col(&source, mid_offset);
        let gutter_width = line_num.to_string().len();
        let expected_caret_pos = gutter_width + 3 + (col - 1);

        let caret_line = output_lines[2];
        let caret_pos = caret_line.find('^').unwrap();

        prop_assert_eq!(
            caret_pos, expected_caret_pos,
            "Multi-line source: offset={}, line={}, col={}, expected caret at {}, got {}",
            mid_offset, line_num, col, expected_caret_pos, caret_pos
        );
    }
}
