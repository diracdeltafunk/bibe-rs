//! Reproduced biber bugs (docs/compat.md §2). Each quirk is on by default
//! (biber's behaviour) and can be switched off with `--fix-quirks`.
//!
//! Only the quirks the configuration front end needs are listed so far; later
//! milestones add theirs.

/// A reproduced biber bug.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Quirk {
    /// BB-021 (`entry-scoped-options-ignored`): `getblxoption(undef, …)`
    /// cannot see per-entry options.
    EntryScopedOptionsIgnored,
    /// BB-053 (`blx-defaults-override-conf`): the hard-coded biblatex
    /// defaults are applied after biber.conf and overwrite its values.
    BlxDefaultsOverrideConf,
    /// BB-083 (`conf-labelalphanametemplate-last`): in biber.conf only the
    /// last `labelalphanametemplate` survives.
    ConfLabelalphanameLastWins,
    /// BB-052 (`conf-sourcemaps-dropped`): biber.conf user source maps are
    /// dropped when the `.bcf` has a `<sourcemap>` without driver/style maps.
    ConfSourcemapsDropped,
    /// BB-082 (`uniquename-base-check-global`): the "at least one base part"
    /// check is done across all uniquename templates, not per template.
    UniquenameBaseCheckGlobal,
    /// BB-054 (`duplicate-datalists`): repeated identical `<datalist>`s are
    /// not de-duplicated.
    DuplicateDatalists,
    /// BB-030 (`smap-entrytarget-source`): `map_field_source` →
    /// `map_field_target` on an `map_entrytarget` reads the value from the
    /// entry being mapped instead of the target.
    SmapEntrytargetSource,
    /// BB-031 (`smap-starnocited`): `map_entrykey_starnocited` applies to
    /// every entry when there is no `\nocite{*}`.
    SmapStarnocited,
    /// BB-034 (`smap-mapuniq-outside-foreach`): `$MAPUNIQ`/`$MAPUNIQVAL`
    /// stay literal outside `map_foreach` maps.
    SmapMapuniqOutsideForeach,
    /// BB-017 (`sortkey-eval`): sort keys are built by string-`eval` of
    /// `getSortKey(q{$value})`, so unbalanced braces or a trailing backslash
    /// give an undef key (sorts first) and `\\`, `\{`, `\}` are unescaped.
    SortkeyEval,
    /// BB-018 (`filter-readds-sets`): after list filtering, every set and set
    /// member is re-added if any set passed the filter.
    FilterReaddsSets,
    /// BB-019 (`per-name-option-leak`): per-name/per-namelist option
    /// overrides leak to the following names.
    PerNameOptionLeak,
    /// BB-042 (`sort-column-type`): a sort column's type comes from its first
    /// item only, and `labelyear`/`labelmonth`/`labelday` columns are strings.
    SortColumnType,
    /// BB-055 (`zero-is-empty`): field values equal to `0` are treated as
    /// missing (sorting).
    ZeroIsEmpty,
    /// BB-015 (`bbl-comma-cleanup`): the `.bbl` "dangling comma" cleanup
    /// `s/,(?:\n\s+)?\}\}/}}/g` also rewrites `,}}` in field data.
    BblCommaCleanup,
    /// BB-016 (`rangelen-zero-digit`): `rangelen` treats a `0` digit of an
    /// abbreviated range end as missing (`105-10` → 11 pages).
    RangelenZeroDigit,
    /// BB-028 (`datefield-helper-mutation`): the `.bbl` writer strips `date`
    /// from the shared data-model date-field list in place, so a custom date
    /// field whose stem ends in `date` loses its flags after the first entry.
    DatefieldHelperMutation,
    /// BB-027 (`workuniq-skipbib-token`): work uniqueness is skipped only for
    /// entries whose `options` field holds the literal token `skipbib`
    /// (`skipbib=true`, `dataonly` and set members are still counted).
    WorkuniqSkipbibToken,
    /// BB-020 (`uniquename-false-skips-entry`): a per-name or per-namelist
    /// `uniquename=false` drops the whole entry from uniquename processing.
    UniquenameFalseSkipsEntry,
    /// BB-095 (`uniquename-minyear-visibility`): uniquename=minyearinit/
    /// minyearfull counts every name as visible (so the minimal scope never
    /// records truncation), while generation uses the normal visibility.
    UniquenameMinyearVisibility,
    /// BB-097 (`uniquelist-minyear-double-count`): uniquelist=minyear counts
    /// each complete name list twice.
    UniquelistMinyearDoubleCount,
    /// BB-094 (`labelcache-field-key`): the labelalpha `v`/`l` width caches
    /// are keyed by field name only, per section, so a second data list or
    /// label part on the same field (other nameparts, name template or
    /// visibility) reuses the first one's cache.
    LabelcacheFieldKey,
    /// BB-096 (`label-global-index-zero`): `substring_width=vf` ignores a
    /// computed fixed width index of 0 (`globalindices || index`).
    LabelGlobalIndexZero,
    /// BB-022/BB-023 (`crossref-date-meta`): crossref inheritance copies the
    /// parent's derived date metadata even when the child's date parts were
    /// kept, and the date-part guard only looks at the main `datesplit`.
    CrossrefDateMeta,
    /// BB-024 (`xdata-name-index`): granular XDATA name references set
    /// 0-based name indices, and `splice` skips re-indexing the last name.
    XdataNameIndex,
    /// BB-025 (`xdata-rerefs`): an XDATA entry's own references are
    /// re-resolved every time it is referenced.
    XdataRerefs,
    /// BB-026 (`shared-namelists`): inherited/XDATA name lists are shared
    /// with the parent (one name-list id), so per-list state is shared.
    SharedNamelists,
    /// BB-067 (`dieondatamodel-no-exit`): `--dieondatamodel` logs errors but
    /// never stops.
    DieondatamodelNoExit,
    /// BB-093 (`clone-endyeardivision`): `Entry::clone` lists `yeardivision`
    /// twice and omits `endyeardivision`, so related-entry clones lose the
    /// end date's season/division.
    CloneEndyeardivision,
    /// (`perlio-encoding-split`; found by `cargo xtask fuzz`, no BB id yet):
    /// with a non-UTF-8 `--output-encoding`, the PerlIO `:encoding` layer
    /// encodes the `.bbl` in 1024-byte chunks of its internal UTF-8, so a
    /// multi-byte character straddling a chunk boundary is written as
    /// `\x{fffd}` sequences (unless the encoding can represent a character
    /// starting with the cut-off prefix).
    PerlioEncodingSplit,
    /// BB-050 (`bltxml-eprint-attrs`): a misplaced parenthesis makes the
    /// biblatexml `eprint` `type`/`class` attributes ignored.
    BltxmlEprintAttrs,
    /// BB-051 (`bltxml-uri-escape`): biblatexml URIs are percent-escaped at
    /// input from NFD text (`urlraw` escaped too, `é` as `e%CC%81`).
    BltxmlUriEscape,
    /// (`bltxml-relatedoptions`, DC-031 report): `relatedoptions` is read
    /// from a `relatedoptions` attribute instead of `options`, so it is
    /// always empty.
    BltxmlRelatedoptions,
    /// (`bltxml-namepart-initial`, DC-031 report): the `initial` attribute
    /// of a simple `namepart` is read from the `name` element instead.
    BltxmlNamepartInitial,
    /// (`bltxml-smap-matchi`, DC-031 report): biblatexml source maps ignore
    /// `map_matchi`/`map_notmatchi` case-insensitivity for matches without
    /// replacement, and `map_notmatchi` is not negated.
    BltxmlSmapMatchi,
    /// (`bltxml-smap-node-string`, DC-031 report): mapping a field to a new
    /// (non-name) field, or using an XPath value as a string, stores the
    /// serialised XML of the source node instead of its text.
    BltxmlSmapNodeString,
    /// BB-047 (`bibtex-winters-typo`): the bibtex writer's year-division
    /// map has `WinterS` for `winterS`, so that season is lost from dates.
    BibtexWinterSTypo,
    /// BB-048 (`bibtex-encoding-question-mark`): with a non-UTF-8 output
    /// encoding the bibtex writer tests `encode(...) =~ /\?/`, so a literal
    /// `?` makes it LaTeX-recode the entry and warn.
    BibtexEncodingQuestionMark,
    /// (`bibtex-unspecified-dates`; new, see the tool package report): the
    /// bibtex writer formats the `XX` of ISO 8601-2 unspecified months and
    /// days with `sprintf('%.2d')`, so `1999-XX` is written as `1999-00`.
    BibtexUnspecifiedDates,
    /// (`bibtex-literal-annotations`; new): the bibtex writer drops the
    /// quotes of literal annotations (`="text"` is written as `=text`).
    BibtexLiteralAnnotations,
    /// (`tool-comments-last-file`; new): in tool mode only the `@comment`s
    /// of the last data source that has any are output.
    ToolCommentsLastFile,
    /// (`tool-drops-preambles`; new): tool-mode (and bibtex) output does
    /// not write the data's `@preamble`s.
    ToolDropsPreambles,
    /// (`libxml2-interleave`; a libxml2 2.9 bug, no BB id yet): biber
    /// validates with libxml2, whose interleave implementation rejects some
    /// valid documents when an interleave operand is itself an interleave
    /// (the generated biblatexml schema; `--validate-bltxml`).
    Libxml2Interleave,
    /// (`unicode-output-nul-check`; M4 io package, no BB id yet): biber
    /// tests "can this entry be encoded" with `encode($enc, $s, sub {"\0"})
    /// =~ /\0/`, which is true for nearly every string in UTF-16/UTF-32/UCS-2
    /// (their code units contain NUL bytes), so with such an
    /// `--output-encoding` every entry is recoded into LaTeX macros with a
    /// "cannot be encoded" WARN.
    UnicodeOutputNulCheck,
    /// (`utf16-chunk-surrogate`; M4 io package, no BB id yet): a UTF-16 data
    /// source is read through PerlIO in 1024-byte chunks, and Encode::Unicode
    /// croaks "UTF-16:Partial character" when a surrogate pair straddles a
    /// chunk boundary, so the file cannot be read.
    Utf16ChunkSurrogate,
}

impl Quirk {
    /// All quirks.
    pub const ALL: &'static [Quirk] = &[
        Quirk::EntryScopedOptionsIgnored,
        Quirk::BlxDefaultsOverrideConf,
        Quirk::ConfLabelalphanameLastWins,
        Quirk::ConfSourcemapsDropped,
        Quirk::UniquenameBaseCheckGlobal,
        Quirk::DuplicateDatalists,
        Quirk::SmapEntrytargetSource,
        Quirk::SmapStarnocited,
        Quirk::SmapMapuniqOutsideForeach,
        Quirk::SortkeyEval,
        Quirk::FilterReaddsSets,
        Quirk::PerNameOptionLeak,
        Quirk::SortColumnType,
        Quirk::ZeroIsEmpty,
        Quirk::BblCommaCleanup,
        Quirk::RangelenZeroDigit,
        Quirk::DatefieldHelperMutation,
        Quirk::WorkuniqSkipbibToken,
        Quirk::UniquenameFalseSkipsEntry,
        Quirk::UniquenameMinyearVisibility,
        Quirk::UniquelistMinyearDoubleCount,
        Quirk::LabelcacheFieldKey,
        Quirk::LabelGlobalIndexZero,
        Quirk::CrossrefDateMeta,
        Quirk::XdataNameIndex,
        Quirk::XdataRerefs,
        Quirk::SharedNamelists,
        Quirk::DieondatamodelNoExit,
        Quirk::CloneEndyeardivision,
        Quirk::PerlioEncodingSplit,
        Quirk::BltxmlEprintAttrs,
        Quirk::BltxmlUriEscape,
        Quirk::BltxmlRelatedoptions,
        Quirk::BltxmlNamepartInitial,
        Quirk::BltxmlSmapMatchi,
        Quirk::BltxmlSmapNodeString,
        Quirk::BibtexWinterSTypo,
        Quirk::BibtexEncodingQuestionMark,
        Quirk::BibtexUnspecifiedDates,
        Quirk::BibtexLiteralAnnotations,
        Quirk::ToolCommentsLastFile,
        Quirk::ToolDropsPreambles,
        Quirk::Libxml2Interleave,
        Quirk::UnicodeOutputNulCheck,
        Quirk::Utf16ChunkSurrogate,
    ];

    /// The `--fix-quirks` switch name.
    pub fn name(self) -> &'static str {
        match self {
            Quirk::EntryScopedOptionsIgnored => "entry-scoped-options-ignored",
            Quirk::BlxDefaultsOverrideConf => "blx-defaults-override-conf",
            Quirk::ConfLabelalphanameLastWins => "conf-labelalphanametemplate-last",
            Quirk::ConfSourcemapsDropped => "conf-sourcemaps-dropped",
            Quirk::UniquenameBaseCheckGlobal => "uniquename-base-check-global",
            Quirk::DuplicateDatalists => "duplicate-datalists",
            Quirk::SmapEntrytargetSource => "smap-entrytarget-source",
            Quirk::SmapStarnocited => "smap-starnocited",
            Quirk::SmapMapuniqOutsideForeach => "smap-mapuniq-outside-foreach",
            Quirk::SortkeyEval => "sortkey-eval",
            Quirk::FilterReaddsSets => "filter-readds-sets",
            Quirk::PerNameOptionLeak => "per-name-option-leak",
            Quirk::SortColumnType => "sort-column-type",
            Quirk::ZeroIsEmpty => "zero-is-empty",
            Quirk::BblCommaCleanup => "bbl-comma-cleanup",
            Quirk::RangelenZeroDigit => "rangelen-zero-digit",
            Quirk::DatefieldHelperMutation => "datefield-helper-mutation",
            Quirk::WorkuniqSkipbibToken => "workuniq-skipbib-token",
            Quirk::UniquenameFalseSkipsEntry => "uniquename-false-skips-entry",
            Quirk::UniquenameMinyearVisibility => "uniquename-minyear-visibility",
            Quirk::UniquelistMinyearDoubleCount => "uniquelist-minyear-double-count",
            Quirk::LabelcacheFieldKey => "labelcache-field-key",
            Quirk::LabelGlobalIndexZero => "label-global-index-zero",
            Quirk::CrossrefDateMeta => "crossref-date-meta",
            Quirk::XdataNameIndex => "xdata-name-index",
            Quirk::XdataRerefs => "xdata-rerefs",
            Quirk::SharedNamelists => "shared-namelists",
            Quirk::DieondatamodelNoExit => "dieondatamodel-no-exit",
            Quirk::CloneEndyeardivision => "clone-endyeardivision",
            Quirk::PerlioEncodingSplit => "perlio-encoding-split",
            Quirk::BltxmlEprintAttrs => "bltxml-eprint-attrs",
            Quirk::BltxmlUriEscape => "bltxml-uri-escape",
            Quirk::BltxmlRelatedoptions => "bltxml-relatedoptions",
            Quirk::BltxmlNamepartInitial => "bltxml-namepart-initial",
            Quirk::BltxmlSmapMatchi => "bltxml-smap-matchi",
            Quirk::BltxmlSmapNodeString => "bltxml-smap-node-string",
            Quirk::BibtexWinterSTypo => "bibtex-winters-typo",
            Quirk::BibtexEncodingQuestionMark => "bibtex-encoding-question-mark",
            Quirk::BibtexUnspecifiedDates => "bibtex-unspecified-dates",
            Quirk::BibtexLiteralAnnotations => "bibtex-literal-annotations",
            Quirk::ToolCommentsLastFile => "tool-comments-last-file",
            Quirk::ToolDropsPreambles => "tool-drops-preambles",
            Quirk::Libxml2Interleave => "libxml2-interleave",
            Quirk::UnicodeOutputNulCheck => "unicode-output-nul-check",
            Quirk::Utf16ChunkSurrogate => "utf16-chunk-surrogate",
        }
    }

    /// Look up by switch name.
    pub fn from_name(name: &str) -> Option<Quirk> {
        Quirk::ALL.iter().copied().find(|q| q.name() == name)
    }

    fn bit(self) -> u64 {
        1 << (self as u64)
    }
}

/// The set of quirks that are switched on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Quirks(u64);

impl Default for Quirks {
    /// All quirks on: biber's behaviour.
    fn default() -> Self {
        Quirks(Quirk::ALL.iter().fold(0, |a, q| a | q.bit()))
    }
}

impl Quirks {
    /// No quirks reproduced.
    pub fn none() -> Self {
        Quirks(0)
    }

    /// Is `q` reproduced?
    pub fn on(self, q: Quirk) -> bool {
        self.0 & q.bit() != 0
    }

    /// Stop reproducing `q`.
    pub fn fix(&mut self, q: Quirk) {
        self.0 &= !q.bit();
    }

    /// Parse a `--fix-quirks` value (`all` or a comma list of names).
    pub fn apply_fix_list(&mut self, list: &str) -> Result<(), String> {
        for name in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            if name == "all" {
                *self = Quirks::none();
                continue;
            }
            match Quirk::from_name(name) {
                Some(q) => self.fix(q),
                None => return Err(format!("Unknown quirk '{name}'")),
            }
        }
        Ok(())
    }
}
