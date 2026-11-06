//! Natural ("logical") string comparison compatible with Explorer's `StrCmpLogicalW`:
//! case-insensitive, digit runs compared by numeric value, shorter digit runs with
//! leading zeros sort after equal values without them.

use std::cmp::Ordering;

pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut ai = a.chars().peekable();
    let mut bi = b.chars().peekable();
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(ca), Some(cb)) => {
                if ca.is_ascii_digit() && cb.is_ascii_digit() {
                    let mut da = String::new();
                    while let Some(&c) = ai.peek() {
                        if c.is_ascii_digit() { da.push(c); ai.next(); } else { break; }
                    }
                    let mut db = String::new();
                    while let Some(&c) = bi.peek() {
                        if c.is_ascii_digit() { db.push(c); bi.next(); } else { break; }
                    }
                    let ta = da.trim_start_matches('0');
                    let tb = db.trim_start_matches('0');
                    let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                    if ord != Ordering::Equal {
                        return ord;
                    }
                    // Equal numeric value: fewer leading zeros first.
                    let z = da.len().cmp(&db.len());
                    if z != Ordering::Equal {
                        return z;
                    }
                } else {
                    let la = fold(ca);
                    let lb = fold(cb);
                    if la != lb {
                        return rank(la).cmp(&rank(lb)).then(la.cmp(&lb));
                    }
                    ai.next();
                    bi.next();
                }
            }
        }
    }
}

#[inline]
fn fold(c: char) -> char {
    if c.is_ascii() { c.to_ascii_lowercase() } else { c.to_lowercase().next().unwrap_or(c) }
}

/// Explorer sorts punctuation/symbols before digits before letters.
#[inline]
fn rank(c: char) -> u8 {
    if c.is_alphabetic() { 2 } else if c.is_ascii_digit() { 1 } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn basic() {
        assert_eq!(natural_cmp("a2", "a10"), Ordering::Less);
        assert_eq!(natural_cmp("A", "a"), Ordering::Equal);
        assert_eq!(natural_cmp("x01", "x1"), Ordering::Greater);
        assert_eq!(natural_cmp("_a", "a"), Ordering::Less);
        assert_eq!(natural_cmp("1", "a"), Ordering::Less);
        assert_eq!(natural_cmp("img 9", "img 10"), Ordering::Less);
    }
}
