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
