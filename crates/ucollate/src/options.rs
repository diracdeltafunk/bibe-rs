//! Collator options (the tailoring parameters of `Unicode::Collate`).

use crate::Error;

/// Variable weighting (`variable`, alias `alternate`), UTS #10 §4.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Variable {
    /// Variable elements are ignored at levels 1–3 (and at level 4).
    Blanked,
    /// Variable elements are treated like any other element.
    NonIgnorable,
    /// The `Unicode::Collate` default: variable elements are moved to level 4.
    #[default]
    Shifted,
    /// Like [`Variable::Shifted`] but the `FFFF` level-4 weights of other elements are dropped.
    ShiftTrimmed,
}

impl Variable {
    /// The name `Unicode::Collate` stores (lower case).
    pub fn as_str(self) -> &'static str {
        match self {
            Variable::Blanked => "blanked",
            Variable::NonIgnorable => "non-ignorable",
            Variable::Shifted => "shifted",
            Variable::ShiftTrimmed => "shift-trimmed",
        }
    }

    /// Parse a name case-insensitively, as `Unicode::Collate` does. An empty name means
    /// the default (`shifted`).
    pub fn parse(name: &str) -> Result<Variable, Error> {
        match name.to_ascii_lowercase().as_str() {
            "" | "shifted" => Ok(Variable::Shifted),
            "blanked" => Ok(Variable::Blanked),
            "non-ignorable" => Ok(Variable::NonIgnorable),
            "shift-trimmed" => Ok(Variable::ShiftTrimmed),
            _ => Err(Error::InvalidOption(format!(
                "Unicode::Collate unknown variable parameter name: {name}"
            ))),
        }
    }
}

/// Normalisation applied to input strings before collation (`normalization`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Normalization {
    /// `NFD` (also spelled `D`): the `Unicode::Collate` default.
    #[default]
    Nfd,
    /// `NFC` / `C`.
    Nfc,
    /// `NFKD` / `KD`.
    Nfkd,
    /// `NFKC` / `KC`.
    Nfkc,
    /// `"prenormalized"`: no normalisation, but discontiguous contractions are still
    /// matched (input is assumed to be in NFD already). This is what biber uses.
    Prenormalized,
    /// `undef`: no normalisation and no discontiguous contraction matching.
    None,
}

impl Normalization {
    /// Parse a form name as accepted by `Unicode::Normalize::normalize` (case-sensitive)
    /// plus `prenormalized`. `FCD`/`FCC` are valid in Perl but not supported here.
    pub fn parse(name: &str) -> Result<Normalization, Error> {
        Ok(match name {
            "NFD" | "D" => Normalization::Nfd,
            "NFC" | "C" => Normalization::Nfc,
            "NFKD" | "KD" => Normalization::Nfkd,
            "NFKC" | "KC" => Normalization::Nfkc,
            "prenormalized" => Normalization::Prenormalized,
            "FCD" | "FCC" => {
                return Err(Error::Unsupported(format!(
                    "normalization form {name} is not supported"
                )));
            }
            _ => {
                return Err(Error::InvalidOption(format!(
                    "Unicode::Collate unknown normalization form name: {name}"
                )));
            }
        })
    }

    /// The canonical name (`None` for `undef`).
    pub fn as_str(self) -> Option<&'static str> {
        match self {
            Normalization::Nfd => Some("NFD"),
            Normalization::Nfc => Some("NFC"),
            Normalization::Nfkd => Some("NFKD"),
            Normalization::Nfkc => Some("NFKC"),
            Normalization::Prenormalized => Some("prenormalized"),
            Normalization::None => None,
        }
    }
}

/// Options for [`crate::Collator::new`].
///
/// Every field is optional: `None` keeps the value set by the locale tailoring, or the
/// `Unicode::Collate` default. Options are applied like `Unicode::Collate::change()`
/// after the locale has been loaded, so they may override settings that the locale
/// tailoring reserves (e.g. `upper_before_lower` for `da`) — this is how biber uses it.
/// The exceptions are [`CollateOptions::entry`] and [`CollateOptions::suppress`], which are
/// constructor-only in Perl; here they are added to (entry) or replace (suppress) the
/// locale's.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CollateOptions {
    /// Maximum level, 1–4 (default 4).
    pub level: Option<u8>,
    /// Variable weighting (default `shifted`).
    pub variable: Option<Variable>,
    /// Input normalisation (default NFD).
    pub normalization: Option<Normalization>,
    /// Swap the tertiary weights of upper and lower case (UTS #10 §6.6).
    pub upper_before_lower: Option<bool>,
    /// Swap the tertiary weights of katakana and hiragana.
    pub katakana_before_hiragana: Option<bool>,
    /// Levels whose weights are compared backwards (e.g. `[2]` for French accents).
    /// `Some(vec![])` means "no level", like `backwards => undef`.
    pub backwards: Option<Vec<u8>>,
    /// Ignore accents (level-2 differences) while keeping case differences.
    pub ignore_level2: Option<bool>,
    /// Primary weight appended to every Hangul syllable (0 = off).
    pub hangul_terminator: Option<u16>,
    /// Add the code points as a final tie-breaking level.
    pub identical: Option<bool>,
    /// Give U+FFFF the highest primary weight (`FFFF ; [.FFFE.0020.0005]`).
    pub highest_ffff: Option<bool>,
    /// Give U+FFFE the lowest primary weight (`FFFE ; [.0001.0020.0005]`).
    pub minimal_fffe: Option<bool>,
    /// `UCA_Version`; only 43 (the default) is supported.
    pub uca_version: Option<u32>,
    /// Additional collation elements in allkeys syntax (`entry`), applied after the
    /// locale's own entries.
    pub entry: Option<String>,
    /// Code points whose DUCET contractions are suppressed (`suppress`); replaces the
    /// locale's list.
    pub suppress: Option<Vec<u32>>,
}

/// Perl truthiness of an optional scalar.
pub(crate) fn perl_truthy(v: Option<&str>) -> bool {
    matches!(v, Some(s) if !s.is_empty() && s != "0")
}

/// Perl's numification of a string (leading whitespace, optional sign, digits, optional
/// fraction/exponent; anything else is 0), truncated towards zero.
pub(crate) fn perl_int(v: &str) -> i64 {
    let s = v.trim_start();
    let b = s.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        if j < b.len() && b[j].is_ascii_digit() {
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            i = j;
        }
    }
    s[..i].parse::<f64>().map(|f| f.trunc() as i64).unwrap_or(0)
}

pub(crate) fn check_level(v: i64, key: &str) -> Result<u8, Error> {
    if v < 1 {
        Err(Error::InvalidOption(format!(
            "Illegal level {v} (in value for key '{key}') lower than 1."
        )))
    } else if v > 4 {
        Err(Error::InvalidOption(format!(
            "Unsupported level {v} (in value for key '{key}') higher than 4."
        )))
    } else {
        Ok(v as u8)
    }
}

impl CollateOptions {
    /// Options as biber passes them by default for sorting
    /// (`collate_options` = `level => 4, variable => 'non-ignorable',
    /// normalization => 'prenormalized'`) with `upper_before_lower` from `sortupper`.
    pub fn biber_sort(level: u8, upper_before_lower: bool) -> CollateOptions {
        CollateOptions {
            level: Some(level),
            variable: Some(Variable::NonIgnorable),
            normalization: Some(Normalization::Prenormalized),
            upper_before_lower: Some(upper_before_lower),
            ..CollateOptions::default()
        }
    }

    /// Options of biber's `sortinithash` collator
    /// (`Unicode::Collate::Locale->new(locale => …, level => 1)`).
    pub fn biber_sortinit() -> CollateOptions {
        CollateOptions {
            level: Some(1),
            ..CollateOptions::default()
        }
    }

    /// Set an option from a Perl-style key and scalar value, as
    /// `Unicode::Collate::change(key => value)` would interpret it (`None` = `undef`).
    ///
    /// This is how biber's `--collate-options`/`-c key=value` and `collate_options`
    /// config values reach the collator. Keys that `change()` refuses (`entry`, `table`,
    /// `ignoreChar`, `suppress`, …) give [`Error::ChangeNotAllowed`] with the Perl
    /// message; unknown keys are silently ignored, like `change()` does.
    pub fn set_perl(&mut self, key: &str, value: Option<&str>) -> Result<(), Error> {
        match key {
            "level" => {
                let v = value.map(perl_int).unwrap_or(0);
                self.level = Some(check_level(v, "level")?);
            }
            "variable" | "alternate" => {
                self.variable = Some(Variable::parse(value.unwrap_or(""))?);
            }
            "normalization" => {
                self.normalization = Some(match value {
                    None => Normalization::None,
                    Some(v) => Normalization::parse(v)?,
                });
            }
            "upper_before_lower" => self.upper_before_lower = Some(perl_truthy(value)),
            "katakana_before_hiragana" => {
                self.katakana_before_hiragana = Some(perl_truthy(value));
            }
            "ignore_level2" => self.ignore_level2 = Some(perl_truthy(value)),
            "identical" => self.identical = Some(perl_truthy(value)),
            "highestFFFF" => self.highest_ffff = Some(perl_truthy(value)),
            "minimalFFFE" => self.minimal_fffe = Some(perl_truthy(value)),
            "hangul_terminator" => {
                let v = value.map(perl_int).unwrap_or(0);
                self.hangul_terminator = Some(v.clamp(0, 0xFFFF) as u16);
            }
            "backwards" => {
                self.backwards = Some(match value {
                    None => vec![],
                    Some(v) => vec![check_level(perl_int(v), "backwards")?],
                });
            }
            "UCA_Version" => {
                let v = value.map(perl_int).unwrap_or(0);
                if v != 43 {
                    return Err(Error::Unsupported(format!(
                        "UCA_Version {v} is not supported (only 43)"
                    )));
                }
                self.uca_version = Some(43);
            }
            "long_contraction" => {
                if perl_truthy(value) {
                    return Err(Error::Unsupported(
                        "long_contraction is not supported".into(),
                    ));
                }
            }
            "rearrange" | "overrideCJK" | "overrideHangul" | "overrideOut" | "preprocess" => {
                return Err(Error::Unsupported(format!(
                    "{key} (a code/array reference in Perl) is not supported"
                )));
            }
            "entry" | "mapping" | "table" | "maxlength" | "contraction" | "ignoreChar"
            | "ignoreName" | "undefChar" | "undefName" | "rewrite" | "versionTable"
            | "alternateTable" | "backwardsTable" | "forwardsTable" | "rearrangeTable"
            | "variableTable" | "derivCode" | "normCode" | "rearrangeHash"
            | "backwardsFlag" | "suppress" | "suppressHash" | "__useXS" => {
                return Err(Error::ChangeNotAllowed(key.to_string()));
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perl_numbers() {
        assert_eq!(perl_int("2.5"), 2);
        assert_eq!(perl_int(" 3abc"), 3);
        assert_eq!(perl_int("abc"), 0);
        assert_eq!(perl_int("-1"), -1);
        assert_eq!(perl_int("1e1"), 10);
        assert_eq!(perl_int("4e"), 4);
    }

    #[test]
    fn set_perl() {
        let mut o = CollateOptions::default();
        o.set_perl("level", Some("2.5")).unwrap();
        assert_eq!(o.level, Some(2));
        o.set_perl("variable", Some("Non-Ignorable")).unwrap();
        assert_eq!(o.variable, Some(Variable::NonIgnorable));
        assert!(o.set_perl("level", Some("0")).is_err());
        assert!(o.set_perl("variable", Some("bogus")).is_err());
        assert!(matches!(
            o.set_perl("ignoreChar", Some("x")),
            Err(Error::ChangeNotAllowed(_))
        ));
        o.set_perl("nonsense", Some("1")).unwrap();
        o.set_perl("normalization", None).unwrap();
        assert_eq!(o.normalization, Some(Normalization::None));
        assert!(o.set_perl("normalization", Some("nfd")).is_err());
    }
}
