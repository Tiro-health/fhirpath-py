//! Byte-offset → UTF-16 code-unit-offset conversion.
//!
//! The lexer/parser produces byte offsets into the source string, which is the
//! natural unit in Rust. JavaScript strings are UTF-16-indexed, so byte offsets
//! cannot be used directly with `String.prototype.slice`, CodeMirror diagnostics,
//! or any other JS-side text decoration. The wasm boundary uses these helpers
//! to translate spans before serializing them out.

/// Convert a byte offset within `s` into a UTF-16 code-unit offset.
///
/// `byte_offset` should land on a UTF-8 char boundary; if it does not, the
/// returned offset is the UTF-16 position of the next char boundary at or
/// after `byte_offset`. Offsets at or past `s.len()` saturate at the total
/// UTF-16 length.
pub fn byte_to_utf16_offset(s: &str, byte_offset: usize) -> usize {
    let mut utf16 = 0;
    for (i, c) in s.char_indices() {
        if i >= byte_offset {
            return utf16;
        }
        utf16 += c.len_utf16();
    }
    utf16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_offsets_pass_through() {
        let s = "item.where(linkId='abc').answer.value";
        assert_eq!(byte_to_utf16_offset(s, 0), 0);
        assert_eq!(byte_to_utf16_offset(s, 4), 4);
        assert_eq!(byte_to_utf16_offset(s, s.len()), s.len());
    }

    #[test]
    fn two_byte_char_counts_as_one_utf16_unit() {
        // 'ç' is U+00E7: 2 UTF-8 bytes, 1 UTF-16 code unit.
        let s = "façade";
        assert_eq!(s.len(), 7); // 6 chars, but ç is 2 bytes
        assert_eq!(byte_to_utf16_offset(s, 0), 0);
        assert_eq!(byte_to_utf16_offset(s, 2), 2); // 'fa' = 2 bytes / 2 utf16
        assert_eq!(byte_to_utf16_offset(s, 4), 3); // 'faç' = 4 bytes / 3 utf16
        assert_eq!(byte_to_utf16_offset(s, 7), 6); // full string
    }

    #[test]
    fn supplementary_plane_char_counts_as_two_utf16_units() {
        // '😀' is U+1F600: 4 UTF-8 bytes, 2 UTF-16 code units (surrogate pair).
        let s = "a😀b";
        assert_eq!(s.len(), 6);
        assert_eq!(byte_to_utf16_offset(s, 1), 1); // 'a'
        assert_eq!(byte_to_utf16_offset(s, 5), 3); // 'a😀'
        assert_eq!(byte_to_utf16_offset(s, 6), 4); // full string
    }

    #[test]
    fn offset_past_end_saturates() {
        let s = "abc";
        assert_eq!(byte_to_utf16_offset(s, 100), 3);
    }

    #[test]
    fn slicing_a_substring_with_unicode() {
        // Mirrors the issue's acceptance criterion: a JS host doing
        // `expr.slice(start, end)` over UTF-16 indices should land on the
        // intended byte-range substring.
        let expr = "item.where(linkId='façade').answer.value";
        let needle = "linkId='façade'";
        let byte_start = expr.find(needle).unwrap();
        let byte_end = byte_start + needle.len();

        let utf16_start = byte_to_utf16_offset(expr, byte_start);
        let utf16_end = byte_to_utf16_offset(expr, byte_end);

        // Reproduce JS-side slicing by walking UTF-16 code units.
        let utf16_units: Vec<u16> = expr.encode_utf16().collect();
        let sliced = String::from_utf16(&utf16_units[utf16_start..utf16_end]).unwrap();

        assert_eq!(sliced, needle);
    }
}
