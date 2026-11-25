//! Fractional position keys for ordering script lines.
//!
//! Integer `order` columns collide when two people insert at the same spot at once. A line's
//! `position` is instead a string that always sorts between its neighbours' keys, so an insert
//! only creates a new key and never renumbers anyone else's lines.
//!
//! Keys are plain ASCII strings over `0-9A-Za-z` (that is also their byte order), compared
//! with an ordinary byte-wise string comparison: SQLite's default `BINARY` collation, a
//! `COLLATE "C"` column in Postgres, and `<` in JavaScript all agree. Two lines can still end
//! up with the same key (concurrent inserts after the same line); the line id breaks the tie.
//!
//! Invariant: a key is non-empty and does not end in `0` (the smallest digit), which is what
//! guarantees there is always room between two keys.

const DIGITS: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const BASE: usize = DIGITS.len();

#[derive(Debug, PartialEq, Eq)]
pub enum PositionError {
    /// Not a valid key (empty, trailing `0`, or characters outside the alphabet).
    InvalidKey(String),
    /// `before` is not smaller than `after`.
    NotOrdered,
}

impl std::fmt::Display for PositionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidKey(key) => write!(f, "invalid position key {key:?}"),
            Self::NotOrdered => write!(f, "position keys are not in order"),
        }
    }
}

impl std::error::Error for PositionError {}

pub fn is_valid_key(key: &str) -> bool {
    !key.is_empty() && !key.ends_with('0') && key.bytes().all(|b| DIGITS.contains(&b))
}

fn digit_value(byte: u8) -> usize {
    DIGITS.iter().position(|&d| d == byte).expect("validated key")
}

/// Midpoint of `a` (the empty string is the lower bound) and `b` (`None` is the upper bound),
/// read as base-62 fractions. Requires `a < b`.
fn midpoint(a: &str, b: Option<&str>) -> String {
    if let Some(b) = b {
        // Keys share a prefix: it stays, only the remainders need splitting.
        let shared = a
            .bytes()
            .zip(b.bytes())
            .take_while(|(x, y)| x == y)
            .count();
        if shared > 0 {
            return format!("{}{}", &b[..shared], midpoint(&a[shared..], Some(&b[shared..])));
        }
    }
    let low = a.bytes().next().map_or(0, digit_value);
    let high = b.and_then(|b| b.bytes().next()).map_or(BASE, digit_value);
    if high - low > 1 {
        return (DIGITS[(low + high) / 2] as char).to_string();
    }
    match b {
        // Neighbouring digits and `b` goes on: the first digit of `b` alone is below `b`
        // (and above `a`, which continues with a smaller digit or ends).
        Some(b) if b.len() > 1 && high > low => b[..1].to_string(),
        // `a` is empty and `b` starts with the smallest digit: stay in front of the rest.
        Some(b) if high == low => format!("0{}", midpoint("", Some(&b[1..]))),
        // No room above this digit: keep it and look for a midpoint in `a`'s tail.
        _ => format!("{}{}", DIGITS[low] as char, midpoint(a.get(1..).unwrap_or(""), None)),
    }
}

/// A key that sorts strictly between `before` and `after` (either may be absent: the start or
/// the end of the script).
pub fn between(before: Option<&str>, after: Option<&str>) -> Result<String, PositionError> {
    for key in before.iter().chain(after.iter()) {
        if !is_valid_key(key) {
            return Err(PositionError::InvalidKey((*key).to_owned()));
        }
    }
    if let (Some(a), Some(b)) = (before, after) {
        if a >= b {
            return Err(PositionError::NotOrdered);
        }
    }
    Ok(midpoint(before.unwrap_or(""), after))
}

/// Evenly spread keys for a script migrated from integer order: key `i` of `count`, in order.
/// Used once, when old integer `order` values become positions.
pub fn initial(index: usize) -> String {
    format!("k{index:06}V")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insert(keys: &mut Vec<String>, index: usize) {
        let before = index.checked_sub(1).map(|i| keys[i].as_str());
        let after = keys.get(index).map(String::as_str);
        let key = between(before, after).unwrap();
        keys.insert(index, key);
    }

    fn assert_sorted_unique(keys: &[String]) {
        for pair in keys.windows(2) {
            assert!(pair[0] < pair[1], "{:?} !< {:?}", pair[0], pair[1]);
        }
        assert!(keys.iter().all(|key| is_valid_key(key)), "{keys:?}");
    }

    #[test]
    fn first_key_and_both_directions() {
        let first = between(None, None).unwrap();
        assert!(is_valid_key(&first));
        let after = between(Some(&first), None).unwrap();
        let before = between(None, Some(&first)).unwrap();
        assert!(before < first && first < after);
    }

    #[test]
    fn appending_pushing_to_the_front_and_bisecting_stay_ordered() {
        let mut keys = vec![between(None, None).unwrap()];
        for _ in 0..200 {
            let len = keys.len();
            insert(&mut keys, len); // append
        }
        for _ in 0..200 {
            insert(&mut keys, 0); // prepend
        }
        assert_sorted_unique(&keys);
        // Always inserting at the same spot (worst case for key length).
        for _ in 0..300 {
            insert(&mut keys, 200);
        }
        assert_sorted_unique(&keys);
        // Keys stay short enough to be practical (one extra char per ~5-6 splits).
        assert!(keys.iter().map(String::len).max().unwrap() < 120);
    }

    #[test]
    fn deterministic_pseudo_random_inserts_stay_ordered() {
        let mut keys = vec![between(None, None).unwrap()];
        let mut state = 0x2545F4914F6CDD1Du64;
        for _ in 0..2000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let at = (state % (keys.len() as u64 + 1)) as usize;
            insert(&mut keys, at);
        }
        assert_sorted_unique(&keys);
    }

    #[test]
    fn migrated_integer_order_keeps_working() {
        let mut keys: Vec<String> = (1..=30).map(initial).collect();
        assert_sorted_unique(&keys);
        insert(&mut keys, 0);
        insert(&mut keys, 15);
        let len = keys.len();
        insert(&mut keys, len);
        assert_sorted_unique(&keys);
    }

    #[test]
    fn keys_that_start_with_the_smallest_digit_still_have_a_predecessor() {
        let before = between(None, Some("0V")).unwrap();
        assert!(before < "0V".to_owned());
        assert!(is_valid_key(&before));
        let tight = between(Some("1"), Some("11")).unwrap();
        assert!("1" < tight.as_str() && tight.as_str() < "11" && is_valid_key(&tight));
    }

    #[test]
    fn rejects_invalid_and_misordered_keys() {
        assert!(matches!(between(Some(""), None), Err(PositionError::InvalidKey(_))));
        assert!(matches!(between(Some("a0"), None), Err(PositionError::InvalidKey(_))));
        assert!(matches!(between(Some("a-b"), None), Err(PositionError::InvalidKey(_))));
        assert_eq!(between(Some("b"), Some("a")), Err(PositionError::NotOrdered));
        assert_eq!(between(Some("a"), Some("a")), Err(PositionError::NotOrdered));
    }

    #[test]
    fn byte_order_matches_the_alphabet_order() {
        let mut sorted = DIGITS.to_vec();
        sorted.sort_unstable();
        assert_eq!(&sorted[..], &DIGITS[..]);
    }
}
