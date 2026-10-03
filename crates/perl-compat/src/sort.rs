//! Perl string comparison and small list helpers.
//!
//! Perl's `cmp`, `lt`, `sort { $a cmp $b }` (without `use locale`) compare
//! strings by code point. For UTF-8 strings byte order equals code point
//! order, so this is exactly Rust's `str` ordering; [`perl_cmp`] exists so
//! call sites document that they reproduce a perl comparison. Perl's `sort`
//! is a stable merge sort, like `slice::sort_by`.
//!
//! biber call sites: `sort keys %h` throughout (output order of options,
//! fields in tool mode), sort-key comparisons when collation is disabled,
//! `Business::ISBN` publisher-range checks (`ge`/`le`).

use std::cmp::Ordering;

/// Perl `$a cmp $b` (code point order).
#[inline]
pub fn perl_cmp(a: &str, b: &str) -> Ordering {
    a.cmp(b)
}

/// Sort strings as perl's `sort` does (stable, code point order).
pub fn perl_sort<S: AsRef<str>>(v: &mut [S]) {
    v.sort_by(|a, b| perl_cmp(a.as_ref(), b.as_ref()));
}

/// Perl's stable `sort { $a->{k} <=> $b->{k} } @items` (numeric key,
/// `undef` = 0, see [`crate::truth::num_cmp_opt`]).
pub fn sort_by_num_key<T>(items: &mut [T], key: impl Fn(&T) -> Option<&str>) {
    items.sort_by(|a, b| crate::truth::num_cmp_opt(key(a), key(b)));
}

/// `Biber::Utils::reduce_array`: the elements of `a` not in `b`, keeping
/// `a`'s order and duplicates.
pub fn reduce_array<S: AsRef<str> + Clone>(a: &[S], b: &[S]) -> Vec<S> {
    let set: std::collections::HashSet<&str> = b.iter().map(AsRef::as_ref).collect();
    a.iter()
        .filter(|x| !set.contains(x.as_ref()))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmp_is_codepoint() {
        assert_eq!(perl_cmp("Z", "a"), Ordering::Less);
        assert_eq!(perl_cmp("é", "z"), Ordering::Greater);
        assert_eq!(perl_cmp("\u{FFFF}", "\u{10000}"), Ordering::Less);
        let mut v = vec!["b", "B", "a"];
        perl_sort(&mut v);
        assert_eq!(v, ["B", "a", "b"]);
    }
}
