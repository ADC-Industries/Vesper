//! Single-byte code page decoding.

/// Windows-1252 code points for bytes 0x80..=0x9F. The five bytes that
/// cp1252 leaves undefined (0x81, 0x8D, 0x8F, 0x90, 0x9D) map to the C1
/// control with the same value, as Windows' own converter does.
const CP1252_HIGH: [u16; 32] = [
    0x20AC, 0x0081, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, //
    0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0x008D, 0x017D, 0x008F, //
    0x0090, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, //
    0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x009D, 0x017E, 0x0178, //
];

/// Decodes one byte of Windows-1252.
pub(crate) fn cp1252(b: u8) -> char {
    match b {
        0x80..=0x9F => char::from_u32(u32::from(CP1252_HIGH[usize::from(b - 0x80)]))
            .unwrap_or(char::REPLACEMENT_CHARACTER),
        // 0x00..=0x7F is ASCII and 0xA0..=0xFF equals Latin-1.
        _ => char::from(b),
    }
}

#[cfg(test)]
mod tests {
    use super::cp1252;

    #[test]
    fn decodes_cp1252() {
        assert_eq!(cp1252(b'A'), 'A');
        assert_eq!(cp1252(0x92), '\u{2019}');
        assert_eq!(cp1252(0x80), '\u{20AC}');
        assert_eq!(cp1252(0x81), '\u{81}');
        assert_eq!(cp1252(0xE9), '\u{E9}');
    }
}
