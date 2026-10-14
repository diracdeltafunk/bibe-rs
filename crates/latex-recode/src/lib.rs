//! LaTeX ↔ Unicode recoding compatible with biber's `Biber::LaTeX::Recode`
//! (biber 2.22), driven by biber's `recode_data.xml`.
//!
//! # Overview
//!
//! * [`RecodeData`] is a parsed recode data file: the built-in one
//!   ([`RecodeData::builtin`], biber's `lib/Biber/LaTeX/recode_data.xml`,
//!   compiled in by `build.rs`) or a user file given with `--recodedata`
//!   ([`RecodeData::from_xml`]; locating it with `kpsewhich` is the caller's job).
//! * [`Recoder`] is the equivalent of `init_sets($decodecharsset,
//!   $output_safecharsset)`: the decode and encode tables for one pair of set
//!   names (`base`, `full`, `null`, or any set a custom file defines).
//!   [`Recoder::builtin`] caches recoders over the built-in data.
//! * [`Recoder::decode`] is `latex_decode($text)` and [`Recoder::encode`] is
//!   `latex_encode($text)`.
//!
//! # How biber uses these (and what callers must mirror)
//!
//! * **Decoding** happens once per `.bib` file in `parse_decode`
//!   (`Input/file/bibtex.pm:1705`): every field value of every regular entry
//!   is decoded, *except* fields whose name case-insensitively matches a data
//!   model "verbs" field (verbatim and URI fields). Values come from
//!   Text::BibTeX already macro-expanded and in **NFD**; `latex_decode` returns
//!   NFD (the default [`Normalization::Nfd`]). Entries of unknown metatype are
//!   decoded whole (`latex_decode($entry->print_s)`). `@preamble`, `@comment`
//!   and `@string` values are *not* decoded. The decode set is used even when it
//!   is `null`: brace elimination and `\char`/`\hbox` handling still run.
//! * **Encoding** (`latex_recode_output` in `Utils.pm`) is applied to whole
//!   output entry strings (`.bbl` entries, the `\preamble`, bibtex-tool
//!   entries) when `--output-safechars` is set, or when the output encoding
//!   cannot represent the entry (tested on `NFC($s)`). It is applied to the
//!   un-normalised (NFD) internal string; output is NFC'd afterwards.
//!
//! # Exactness
//!
//! Each Perl `s///` is reimplemented as a hand-written scanner that finds the
//! same matches as Perl's backtracking engine (including the variable-length
//! look-behind emulations and the code-block conditionals). Character classes
//! (`\s \w \d \pL \pM`) and `\X` use tables dumped from biber's perl 5.42
//! (Unicode 16.0.0); normalisation uses `unicode-normalization` 0.1.24
//! (Unicode 16.0.0). The crate is tested differentially against biber
//! (oracle O3, see `tools/oracle/probes/latex-recode/`).
//!
//! Known deviations (unavoidable or for unsupported custom data):
//! * `\char` producing a non-scalar code point (a surrogate, or above
//!   U+10FFFF but ≤ `IV_MAX`) yields U+FFFD; Perl keeps the code point.
//! * Custom recode data whose keys contain regex metacharacters are matched
//!   literally (Perl would interpolate them into a regex), and an encode type
//!   whose map becomes empty (or has an empty key) is skipped.
//!
//! # Quirks (BB-029)
//!
//! By default biber's behaviour is reproduced exactly. [`RecodeOptions`]
//! can switch off the two parts of BB-029.

mod decode;
mod encode;
mod tables;
mod trie;
mod unicode;
#[allow(clippy::all)]
mod unicode_tables;
pub mod xmldata;

use std::borrow::Cow;
use std::sync::{Arc, Mutex, OnceLock};

use unicode_normalization::UnicodeNormalization;

pub use xmldata::RawData;

/// The normalisation `latex_decode` applies to its result
/// (`normalize`/`normalization` options of `latex_decode`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Normalization {
    /// `normalize => 0`
    None,
    /// `NFD` (biber's default and only use)
    #[default]
    Nfd,
    /// `NFC`
    Nfc,
    /// `NFKD`
    Nfkd,
    /// `NFKC`
    Nfkc,
}

impl Normalization {
    /// Apply this normalisation form.
    pub fn apply(self, s: &str) -> String {
        match self {
            Normalization::None => s.to_owned(),
            Normalization::Nfd => nfd_fast(s),
            Normalization::Nfc => s.nfc().collect(),
            Normalization::Nfkd => s.nfkd().collect(),
            Normalization::Nfkc => s.nfkc().collect(),
        }
    }
}

fn nfd_fast(s: &str) -> String {
    if s.is_ascii()
        || unicode_normalization::is_nfd_quick(s.chars())
            == unicode_normalization::IsNormalized::Yes
    {
        s.to_owned()
    } else {
        s.nfd().collect()
    }
}

/// Split `s` into Perl 5.42 `\X` clusters (exposed for differential tests).
#[doc(hidden)]
pub fn grapheme_clusters(s: &str) -> Vec<&str> {
    let mut v = Vec::new();
    let mut i = 0;
    while i < s.len() {
        let e = unicode::grapheme_end(s, i);
        v.push(&s[i..e]);
        i = e;
    }
    v
}

/// Switches for biber bugs that `latex_decode` reproduces by default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RecodeOptions {
    /// BB-029(a): biber decodes `\textdollar`, `\textpercent` and
    /// `\textampersand` to raw `$`, `%` and `&`, which break the `.bbl`.
    /// When set, macros whose decoding contains a TeX special character
    /// (`$ % & # _ ^ ~ \ { }`) are left undecoded.
    pub fix_raw_specials: bool,
    /// BB-029(b): punctuation, symbol and greek macros are terminated by
    /// `(?: \{\}|\s+|\b)` with a literal space, so `\textdollar{}5` decodes to
    /// `${}5`. When set, they use the letters' `(?:\{\}|\s+|\b)`.
    pub fix_space_braces: bool,
}

impl RecodeOptions {
    /// biber 2.22's behaviour (the default).
    pub const BIBER: RecodeOptions = RecodeOptions {
        fix_raw_specials: false,
        fix_space_braces: false,
    };
    /// All quirk fixes on.
    pub const FIXED: RecodeOptions = RecodeOptions {
        fix_raw_specials: true,
        fix_space_braces: true,
    };
}

/// Error reading a recode data file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecodeDataError(pub String);

impl std::fmt::Display for RecodeDataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid recode data: {}", self.0)
    }
}

impl std::error::Error for RecodeDataError {}

/// A parsed recode data file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecodeData {
    raw: RawData,
}

mod builtin {
    pub(crate) struct StaticMaps {
        pub ty: &'static str,
        pub sets: &'static [&'static str],
        pub maps: &'static [(&'static str, &'static str, bool, bool)],
    }
    pub(crate) struct StaticData {
        pub blocks: &'static [StaticMaps],
        pub decode_exclude: &'static [&'static str],
        pub encode_exclude: &'static [&'static str],
    }
    include!(concat!(env!("OUT_DIR"), "/builtin_data.rs"));
}

impl RecodeData {
    /// biber 2.22's `recode_data.xml`.
    pub fn builtin() -> &'static RecodeData {
        static DATA: OnceLock<RecodeData> = OnceLock::new();
        DATA.get_or_init(|| {
            let d = &builtin::BUILTIN;
            let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
            RecodeData {
                raw: RawData {
                    blocks: d
                        .blocks
                        .iter()
                        .map(|b| xmldata::RawMaps {
                            ty: b.ty.to_string(),
                            sets: s(b.sets),
                            maps: b
                                .maps
                                .iter()
                                .map(|&(from, to, preferred, raw)| xmldata::RawMap {
                                    from: from.to_string(),
                                    to: to.to_string(),
                                    preferred,
                                    raw,
                                })
                                .collect(),
                        })
                        .collect(),
                    decode_exclude: s(d.decode_exclude),
                    encode_exclude: s(d.encode_exclude),
                },
            }
        })
    }

    /// Parse a recode data file (the `--recodedata` option).
    pub fn from_xml(xml: &str) -> Result<RecodeData, RecodeDataError> {
        xmldata::parse(xml)
            .map(|raw| RecodeData { raw })
            .map_err(RecodeDataError)
    }

    /// The parsed contents.
    pub fn raw(&self) -> &RawData {
        &self.raw
    }
}

/// Decode and encode tables for one (decode set, encode set) pair: the state
/// `Biber::LaTeX::Recode::init_sets` sets up.
#[derive(Clone, Debug)]
pub struct Recoder {
    decode: tables::DecodeTables,
    /// Built on first use for recoders over the built-in data (most runs
    /// never encode), eagerly otherwise.
    encode: OnceLock<tables::EncodeTables>,
    /// The built-in data and encode set `encode` is built from.
    lazy_encode: Option<(&'static RecodeData, String)>,
    opts: RecodeOptions,
}

impl Recoder {
    /// `init_sets($decode_set, $encode_set)` over `data`.
    pub fn new(
        data: &RecodeData,
        decode_set: &str,
        encode_set: &str,
        opts: RecodeOptions,
    ) -> Recoder {
        Recoder {
            decode: tables::build_decode(&data.raw, decode_set, &opts),
            encode: OnceLock::from(tables::build_encode(&data.raw, encode_set)),
            lazy_encode: None,
            opts,
        }
    }

    /// [`Recoder::new`] over the built-in data, building the encode tables
    /// on first use.
    fn new_builtin(decode_set: &str, encode_set: &str, opts: RecodeOptions) -> Recoder {
        let data = RecodeData::builtin();
        Recoder {
            decode: tables::build_decode(&data.raw, decode_set, &opts),
            encode: OnceLock::new(),
            lazy_encode: Some((data, encode_set.to_owned())),
            opts,
        }
    }

    fn encode_tables(&self) -> &tables::EncodeTables {
        self.encode.get_or_init(|| {
            let (data, set) = self
                .lazy_encode
                .as_ref()
                .expect("encode tables are built eagerly unless lazy_encode is set");
            tables::build_encode(&data.raw, set)
        })
    }

    /// A shared recoder over the built-in data (cached per argument tuple).
    pub fn builtin(decode_set: &str, encode_set: &str, opts: RecodeOptions) -> Arc<Recoder> {
        type Cache = Mutex<Vec<((String, String, RecodeOptions), Arc<Recoder>)>>;
        static CACHE: OnceLock<Cache> = OnceLock::new();
        let cache = CACHE.get_or_init(|| Mutex::new(Vec::new()));
        let mut c = cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, r)) = c
            .iter()
            .find(|((d, e, o), _)| d == decode_set && e == encode_set && *o == opts)
        {
            return Arc::clone(r);
        }
        let r = Arc::new(Recoder::new_builtin(decode_set, encode_set, opts));
        c.push((
            (decode_set.to_owned(), encode_set.to_owned(), opts),
            Arc::clone(&r),
        ));
        r
    }

    /// The options this recoder was built with.
    pub fn options(&self) -> RecodeOptions {
        self.opts
    }

    /// `latex_decode($text)`: LaTeX macros → Unicode, result in NFD.
    pub fn decode(&self, text: &str) -> String {
        self.decode_with(text, Normalization::Nfd)
    }

    /// `latex_decode($text, normalize => …, normalization => …)`.
    pub fn decode_with(&self, text: &str, norm: Normalization) -> String {
        let s = decode::decode_raw(&self.decode, &self.opts, text);
        match norm {
            Normalization::None => s,
            Normalization::Nfd => {
                if s.is_ascii()
                    || unicode_normalization::is_nfd_quick(s.chars())
                        == unicode_normalization::IsNormalized::Yes
                {
                    s
                } else {
                    s.nfd().collect()
                }
            }
            n => n.apply(&s),
        }
    }

    /// [`decode`](Self::decode), borrowing `text` when the result is `text`
    /// itself (nothing to decode and already NFD).
    pub fn decode_cow<'t>(&self, text: &'t str) -> Cow<'t, str> {
        let unchanged = !decode::needs_work(text)
            && (text.is_ascii()
                || unicode_normalization::is_nfd_quick(text.chars())
                    == unicode_normalization::IsNormalized::Yes);
        if unchanged {
            Cow::Borrowed(text)
        } else {
            Cow::Owned(self.decode(text))
        }
    }

    /// Whether [`decode`](Self::decode) can change `text` other than by
    /// normalisation (a cheap pre-check).
    pub fn decode_may_change(&self, text: &str) -> bool {
        decode::needs_work(text)
    }

    /// `latex_encode($text)`: Unicode → LaTeX macros.
    pub fn encode(&self, text: &str) -> String {
        encode::encode(self.encode_tables(), text)
    }
}
