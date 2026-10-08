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
        Quirk::UniquenameFalseSkipsEntry,
        Quirk::UniquenameMinyearVisibility,
        Quirk::UniquelistMinyearDoubleCount,
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
            Quirk::UniquenameFalseSkipsEntry => "uniquename-false-skips-entry",
            Quirk::UniquenameMinyearVisibility => "uniquename-minyear-visibility",
            Quirk::UniquelistMinyearDoubleCount => "uniquelist-minyear-double-count",
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
