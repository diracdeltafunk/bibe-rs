//! Every character class biber's users can write that we have perl's exact
//! set for (from perl 5.42 itself, see `unicode_exhaustive.rs`) must match
//! the same code points through the PCRE2 layer.

use perl_compat::regex::PerlRegex;
use perl_compat::unicode::Prop;

#[test]
fn pcre2_classes_match_perl() {
    let mut bad = Vec::new();
    for &p in Prop::ALL {
        let syn = p.perl_syntax();
        let re = match PerlRegex::new(&format!("\\A{syn}\\z"), false) {
            Ok(r) => r,
            Err(e) => {
                bad.push(format!("{syn}: does not compile: {e}"));
                continue;
            }
        };
        let mut diffs = Vec::new();
        for c in (0..=0x10FFFFu32).filter_map(char::from_u32) {
            let mut b = [0u8; 4];
            let s = c.encode_utf8(&mut b);
            if re.is_match(s).unwrap() != p.contains(c) {
                diffs.push(c as u32);
            }
        }
        if !diffs.is_empty() {
            bad.push(format!(
                "{syn}: {} code points differ, e.g. {:X?}",
                diffs.len(),
                &diffs[..diffs.len().min(12)]
            ));
        }
    }
    assert!(bad.is_empty(), "\n{}", bad.join("\n"));
}

#[test]
fn negated_and_bracketed_forms() {
    let mut bad = Vec::new();
    for (p, e, ne) in [(Prop::PerlWord, 'w', 'W'), (Prop::PerlSpace, 's', 'S'), (Prop::PerlHSpace, 'h', 'H')] {
        let cases: Vec<(String, Box<dyn Fn(char) -> bool>)> = vec![
            (format!("\\{ne}"), Box::new(move |c| !p.contains(c))),
            (format!("[\\{e}]"), Box::new(move |c| p.contains(c))),
            (format!("[\\{ne}]"), Box::new(move |c| !p.contains(c))),
            (format!("[^\\{e}]"), Box::new(move |c| !p.contains(c))),
            (format!("[^\\{ne}]"), Box::new(move |c| p.contains(c))),
            (format!("[a\\{ne}]"), Box::new(move |c| c == 'a' || !p.contains(c))),
            (format!("[^a\\{ne}]"), Box::new(move |c| c != 'a' && p.contains(c))),
        ];
        for (pat, want) in cases {
            let re = PerlRegex::new(&format!("\\A{pat}\\z"), false).unwrap();
            let n = (0..=0x10FFFFu32)
                .filter_map(char::from_u32)
                .filter(|&c| {
                    let mut b = [0u8; 4];
                    re.is_match(c.encode_utf8(&mut b)).unwrap() != want(c)
                })
                .count();
            if n > 0 {
                bad.push(format!("{pat}: {n} differ"));
            }
        }
    }
    // \b with perl's \w: U+00B2 is not a word character in perl (PCRE2 says it is)
    let re = PerlRegex::new(r"a\b", false).unwrap();
    assert!(re.is_match("a\u{B2}").unwrap());
    assert!(re.is_match("a-").unwrap());
    assert!(bad.is_empty(), "\n{}", bad.join("\n"));
}
