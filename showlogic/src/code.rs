//! Caller codes: what a person types to become show caller.

use uuid::Uuid;

pub const MIN_CODE_CHARS: usize = 6;
pub const MAX_CODE_CHARS: usize = 32;
/// Length of a generated code when the host does not choose one.
pub const DEFAULT_GENERATED_CHARS: usize = 8;

#[derive(Debug, PartialEq, Eq)]
pub enum CodeError {
    TooShort,
    TooLong,
    /// Spaces, control characters and anything outside printable ASCII.
    BadCharacter,
}

impl std::fmt::Display for CodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort => write!(f, "The code needs at least {MIN_CODE_CHARS} characters"),
            Self::TooLong => write!(f, "The code can be at most {MAX_CODE_CHARS} characters"),
            Self::BadCharacter => write!(f, "Use letters, digits and symbols without spaces"),
        }
    }
}

/// 6 to 32 printable ASCII characters. Special characters are welcome but never required.
pub fn validate(code: &str) -> Result<(), CodeError> {
    if !code.bytes().all(|b| (b'!'..=b'~').contains(&b)) {
        return Err(CodeError::BadCharacter);
    }
    match code.len() {
        n if n < MIN_CODE_CHARS => Err(CodeError::TooShort),
        n if n > MAX_CODE_CHARS => Err(CodeError::TooLong),
        _ => Ok(()),
    }
}

fn is_hex(code: &str) -> bool {
    !code.is_empty() && code.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Equal length-independent comparison, so timing says nothing about how much was right.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let mut difference = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        difference |= usize::from(a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0));
    }
    difference == 0
}

/// Exact match, except that a purely hexadecimal code ignores case (`3FA9C1` = `3fa9c1`).
pub fn matches(expected: &str, given: &str) -> bool {
    if is_hex(expected) {
        constant_time_eq(expected.to_ascii_lowercase().as_bytes(), given.to_ascii_lowercase().as_bytes())
    } else {
        constant_time_eq(expected.as_bytes(), given.as_bytes())
    }
}

/// Random hex digits from the operating system's random source (through UUID v4).
pub fn random_hex(length: usize) -> String {
    let mut out = String::with_capacity(length);
    while out.len() < length {
        let block = Uuid::new_v4().simple().to_string();
        // A v4 UUID fixes the version digit (index 12) and limits the variant digit (index 16).
        out.extend(block.chars().enumerate().filter(|(i, _)| *i != 12 && *i != 16).map(|(_, c)| c));
    }
    out.truncate(length);
    out
}

/// A new random hexadecimal code, 6 to 32 characters long.
pub fn generate(length: usize) -> String {
    random_hex(length.clamp(MIN_CODE_CHARS, MAX_CODE_CHARS)).to_ascii_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_is_six_to_thirty_two() {
        assert_eq!(validate("12345"), Err(CodeError::TooShort));
        assert_eq!(validate("123456"), Ok(()));
        assert_eq!(validate(&"a".repeat(32)), Ok(()));
        assert_eq!(validate(&"a".repeat(33)), Err(CodeError::TooLong));
        assert_eq!(validate(""), Err(CodeError::TooShort));
    }

    #[test]
    fn symbols_are_allowed_but_never_required() {
        for ok in ["abcdef", "000000", "3FA9C1D27B", "P@ss!w#rd", "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~", "0123456789abcdef0123456789ABCDEF"] {
            assert_eq!(validate(ok), Ok(()), "{ok}");
        }
        for bad in ["abc def", "abcdef ", " abcdef", "abc\tdef", "abcdéf", "abcdef\u{7f}", "abc\ndef", "🔑🔑🔑🔑🔑🔑"] {
            assert_eq!(validate(bad), Err(CodeError::BadCharacter), "{bad:?}");
        }
    }

    #[test]
    fn hex_codes_ignore_case_and_everything_else_is_exact() {
        assert!(matches("3FA9C1", "3fa9c1"));
        assert!(matches("deadbeef", "DEADBEEF"));
        assert!(!matches("Secret1", "secret1"), "not hex: case matters");
        assert!(!matches("abc!ef", "ABC!EF"));
        assert!(!matches("123456", "123457"));
        assert!(!matches("123456", "1234567"));
        assert!(!matches("123456", ""));
        assert!(matches("p@ss-w0rd!", "p@ss-w0rd!"));
    }

    #[test]
    fn generated_codes_are_random_hex_of_the_requested_length() {
        for length in [6, 8, 17, 32] {
            let code = generate(length);
            assert_eq!(code.len(), length);
            assert!(is_hex(&code) && validate(&code).is_ok(), "{code}");
        }
        assert_eq!(generate(1).len(), MIN_CODE_CHARS, "clamped up");
        assert_eq!(generate(99).len(), MAX_CODE_CHARS, "clamped down");
        assert_ne!(generate(32), generate(32));
        // Every hex digit shows up over many draws: nothing is stuck at the UUID's fixed nibbles.
        let mut seen = [false; 16];
        for c in random_hex(2000).chars() {
            seen[c.to_digit(16).unwrap() as usize] = true;
        }
        assert!(seen.iter().all(|&s| s));
    }

    #[test]
    fn constant_time_comparison_is_still_a_correct_comparison() {
        assert!(constant_time_eq(b"", b""));
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(!constant_time_eq(b"", b"a"));
    }
}
