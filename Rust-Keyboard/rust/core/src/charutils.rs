//! The handful of Unicode helpers the matcher needs.
//!
//! The original engine ships a ~1000 line `CharUtils` generated from ICU data.
//! We only need three operations, and we only need them to be *consistent*: a
//! table of the accented letters that appear in Latin dictionaries, folded to
//! their base letter.

/// `(accented, base)` pairs, sorted by the first element for binary search.
static ACCENT_FOLD: &[(i32, i32)] = &[
    (0x00C0, 0x41), (0x00C1, 0x41), (0x00C2, 0x41), (0x00C3, 0x41), (0x00C4, 0x41),
    (0x00C5, 0x41), (0x00C7, 0x43), (0x00C8, 0x45), (0x00C9, 0x45), (0x00CA, 0x45),
    (0x00CB, 0x45), (0x00CC, 0x49), (0x00CD, 0x49), (0x00CE, 0x49), (0x00CF, 0x49),
    (0x00D1, 0x4E), (0x00D2, 0x4F), (0x00D3, 0x4F), (0x00D4, 0x4F), (0x00D5, 0x4F),
    (0x00D6, 0x4F), (0x00D9, 0x55), (0x00DA, 0x55), (0x00DB, 0x55), (0x00DC, 0x55),
    (0x00DD, 0x59), (0x00E0, 0x61), (0x00E1, 0x61), (0x00E2, 0x61), (0x00E3, 0x61),
    (0x00E4, 0x61), (0x00E5, 0x61), (0x00E7, 0x63), (0x00E8, 0x65), (0x00E9, 0x65),
    (0x00EA, 0x65), (0x00EB, 0x65), (0x00EC, 0x69), (0x00ED, 0x69), (0x00EE, 0x69),
    (0x00EF, 0x69), (0x00F1, 0x6E), (0x00F2, 0x6F), (0x00F3, 0x6F), (0x00F4, 0x6F),
    (0x00F5, 0x6F), (0x00F6, 0x6F), (0x00F9, 0x75), (0x00FA, 0x75), (0x00FB, 0x75),
    (0x00FC, 0x75), (0x00FD, 0x79), (0x00FF, 0x79), (0x0101, 0x61), (0x0103, 0x61),
    (0x0105, 0x61), (0x0107, 0x63), (0x010D, 0x63), (0x0119, 0x65), (0x0142, 0x6C),
];

/// Lower-cases a code point (ASCII + Latin-1 + Latin Extended-A).
#[inline]
pub fn to_lower(code_point: i32) -> i32 {
    if (0x41..=0x5A).contains(&code_point) {
        return code_point + 0x20;
    }
    if (0x00C0..=0x00DE).contains(&code_point) && code_point != 0x00D7 {
        return code_point + 0x20;
    }
    if (0x0100..=0x0137).contains(&code_point) && code_point % 2 == 0 {
        return code_point + 1;
    }
    if (0x0139..=0x0148).contains(&code_point) && code_point % 2 == 1 {
        return code_point + 1;
    }
    if (0x014A..=0x0177).contains(&code_point) && code_point % 2 == 0 {
        return code_point + 1;
    }
    code_point
}

/// Upper-cases a code point (ASCII + Latin-1 + Latin Extended-A).
#[inline]
pub fn to_upper(code_point: i32) -> i32 {
    if (0x61..=0x7A).contains(&code_point) {
        return code_point - 0x20;
    }
    if (0x00E0..=0x00FE).contains(&code_point) && code_point != 0x00F7 {
        return code_point - 0x20;
    }
    if (0x0101..=0x0138).contains(&code_point) && code_point % 2 == 1 {
        return code_point - 1;
    }
    if (0x013A..=0x0149).contains(&code_point) && code_point % 2 == 0 {
        return code_point - 1;
    }
    if (0x014B..=0x0178).contains(&code_point) && code_point % 2 == 1 {
        return code_point - 1;
    }
    code_point
}

#[inline]
pub fn is_upper(code_point: i32) -> bool {
    (0x41..=0x5A).contains(&code_point) || (0x00C0..=0x00DE).contains(&code_point)
}

/// Folds an accented letter to its unaccented base letter.
pub fn to_base_code_point(code_point: i32) -> i32 {
    let mut low = 0usize;
    let mut high = ACCENT_FOLD.len();
    while low < high {
        let mid = (low + high) / 2;
        let entry = ACCENT_FOLD[mid];
        if entry.0 == code_point {
            return entry.1;
        } else if entry.0 < code_point {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    code_point
}

#[inline]
pub fn to_base_lower(code_point: i32) -> i32 {
    to_base_code_point(to_lower(code_point))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_case() {
        assert_eq!(to_lower(0x41), 0x61);
        assert_eq!(to_upper(0x61), 0x41);
        assert_eq!(to_lower(0x00C9), 0x00E9); // É -> é
        assert_eq!(to_upper(0x00E9), 0x00C9);
    }

    #[test]
    fn folds_accents() {
        assert_eq!(to_base_code_point(0x00E9), 0x65); // é -> e
        assert_eq!(to_base_code_point(0x00FC), 0x75); // ü -> u
        assert_eq!(to_base_code_point(0x61), 0x61); // unaffected
    }
}
