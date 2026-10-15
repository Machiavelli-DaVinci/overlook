//! IMAP sequence sets (`1:5,8,10:*`) as sorted UID lists, and back.
//!
//! Folder cursors keep the UIDs Overlook knows about as a sequence-set string: compact in
//! the checkpoint, and directly usable as the argument of `UID FETCH` and `UID SEARCH`.

use std::collections::BTreeSet;

/// Sorted, de-duplicated UIDs as the shortest equivalent sequence set.
pub fn encode(uids: &BTreeSet<u32>) -> String {
    let mut out = String::new();
    let mut run: Option<(u32, u32)> = None;
    let flush = |out: &mut String, run: Option<(u32, u32)>| {
        if let Some((a, b)) = run {
            if !out.is_empty() {
                out.push(',');
            }
            if a == b {
                out.push_str(&a.to_string());
            } else {
                out.push_str(&format!("{a}:{b}"));
            }
        }
    };
    for &uid in uids {
        match run {
            Some((a, b)) if uid == b + 1 => run = Some((a, uid)),
            other => {
                flush(&mut out, other);
                run = Some((uid, uid));
            }
        }
    }
    flush(&mut out, run);
    out
}

/// Parses a sequence set. `*` is not meaningful without a mailbox and is ignored; malformed
/// pieces are skipped rather than failing, since the input is our own checkpoint.
pub fn decode(set: &str) -> BTreeSet<u32> {
    let mut out = BTreeSet::new();
    for piece in set.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        match piece.split_once(':') {
            Some((a, b)) => {
                if let (Ok(a), Ok(b)) = (a.parse::<u32>(), b.parse::<u32>()) {
                    let (lo, hi) = (a.min(b), a.max(b));
                    // Guard against a corrupt checkpoint expanding into millions of entries.
                    if hi - lo < 1_000_000 {
                        out.extend(lo..=hi);
                    }
                }
            }
            None => {
                if let Ok(n) = piece.parse::<u32>() {
                    out.insert(n);
                }
            }
        }
    }
    out
}

/// Splits a set into chunks whose encoded form stays comfortably inside one command line.
pub fn chunks(uids: &BTreeSet<u32>, per_chunk: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = BTreeSet::new();
    for &uid in uids {
        current.insert(uid);
        if current.len() >= per_chunk {
            out.push(encode(&current));
            current.clear();
        }
    }
    if !current.is_empty() {
        out.push(encode(&current));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_ranges_and_singles() {
        let set: BTreeSet<u32> = [1, 2, 3, 5, 8, 9, 10, 20].into_iter().collect();
        assert_eq!(encode(&set), "1:3,5,8:10,20");
        assert_eq!(decode("1:3,5,8:10,20"), set);
        assert_eq!(decode("10:8, 5 ,x,3:3"), [3, 5, 8, 9, 10].into_iter().collect());
        assert_eq!(encode(&BTreeSet::new()), "");
        assert!(decode("").is_empty());
        assert!(decode("1:999999999").is_empty(), "absurd ranges are dropped");
    }

    #[test]
    fn chunks_keep_order() {
        let set: BTreeSet<u32> = (1..=7).collect();
        assert_eq!(chunks(&set, 3), vec!["1:3", "4:6", "7"]);
    }
}
