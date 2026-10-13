//! The `.bbl` writer (`Biber::Output::bbl`, bbl.pm), producing exactly
//! biber's text: see `docs/research/05-output-and-testing.md` §2 for the
//! format specification.
//!
//! biber builds each entry's text once per section (`set_output_entry`,
//! bbl.pm:196) with `<BDS>…</BDS>` placeholders for the values that depend
//! on the data list (labelalpha, extradate, sortinit, uniquename, name
//! hashes, …), and fills them in per list with regex substitutions
//! (`Biber::DataList::instantiate_entry`, DataList.pm:1813). Here the
//! per-section part is an [`EntryTemplate`] of structured items, and
//! [`instantiate_entry`] renders it for one list directly. Observable
//! effects of the placeholder machinery are reproduced explicitly:
//!
//! - unreplaced placeholder lines are dropped (DataList.pm:2225);
//! - placeholders removed from option lists take their following comma
//!   with them, so removing the *last* one leaves a trailing comma
//!   ([`join_removable`]);
//! - a placeholder's value is decided by its first occurrence in the
//!   entry (substitutions are global over the entry string);
//! - the "dangling comma" cleanup `s/,(?:\n\s+)?\}\}/}}/g`
//!   (DataList.pm:2222) runs over the whole entry, user data included
//!   (BB-015, [`Quirk::BblCommaCleanup`]).

use std::collections::{BTreeMap, HashMap};

use md5::{Digest, Md5};
use perl_compat::encoding::{Encoding, Lookup, lookup};
use perl_compat::grapheme::gcstring;
use perl_compat::textwrap;
use perl_compat::truth::PerlNum;
use perl_compat::unicode::{Prop, is_perl_space, nfc, nfkd};
use perl_compat::{lc, looks_like_number, perl_truthy};
use serde_json::Value as J;

use crate::biber_error;
use crate::config::blx::{OptValue, Scope, map_boolean_tostring_raw};
use crate::datamodel::DataModel;
use crate::log::Fatal;
use crate::model::{
    DataList, Entry, Name, NameArena, NameId, NameListId, PerlData, SectionState, Value,
};
use crate::pipeline::{Ctx, Processed};
use crate::quirks::Quirk;

/// `$BBL_VERSION` (Constants.pm:53).
pub const BBL_VERSION: &str = "3.3";

/// The `.bbl` header (bbl.pm:42-61).
fn head() -> String {
    format!(
        "% $ biblatex auxiliary file $\n\
         % $ biblatex bbl format version {BBL_VERSION} $\n\
         % Do not modify the above lines!\n\
         %\n\
         % This is an auxiliary file used by the 'biblatex' package.\n\
         % This file may safely be deleted. It will be recreated by\n\
         % biber as required.\n\
         %\n\
         \\begingroup\n\
         \\makeatletter\n\
         \\@ifundefined{{ver@biblatex.sty}}\n\
         \x20 {{\\@latex@error\n\
         \x20    {{Missing 'biblatex' package}}\n\
         \x20    {{The bibliography requires the 'biblatex' package.}}\n\
         \x20     \\aftergroup\\endinput}}\n\
         \x20 {{}}\n\
         \\endgroup\n\
         \n"
    )
}

// ---------------------------------------------------------------------------
// Small Perl-shaped helpers
// ---------------------------------------------------------------------------

/// Perl truthiness of an entry field value (references are true).
fn val_truthy(v: Option<&Value>) -> bool {
    match v {
        None => false,
        Some(Value::Str(s)) => perl_truthy(s),
        Some(Value::Other(J::Null)) => false,
        Some(Value::Other(J::String(s))) => perl_truthy(s),
        Some(_) => true,
    }
}

/// A field value interpolated into a string.
fn val_string(v: Option<&Value>) -> String {
    match v {
        None => String::new(),
        Some(Value::Str(s)) => s.clone(),
        Some(Value::List(l)) => l.join(","),
        Some(Value::Other(J::String(s))) => s.clone(),
        Some(Value::Other(o)) => o.to_string(),
        Some(Value::Ranges(_)) | Some(Value::Names(_)) => String::new(),
    }
}

/// Perl truthiness of untyped data (`undef` is false, references true).
fn pd_truthy(v: Option<&PerlData>) -> bool {
    match v {
        None | Some(J::Null) => false,
        Some(J::String(s)) => perl_truthy(s),
        Some(J::Bool(b)) => *b,
        Some(J::Number(n)) => perl_truthy(&n.to_string()),
        Some(_) => true,
    }
}

/// Untyped scalar interpolated into a string (`undef` → empty).
fn pd_string(v: Option<&PerlData>) -> String {
    match v {
        None | Some(J::Null) => String::new(),
        Some(J::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

/// `$h->{a}{b}…` on untyped data.
fn pd_path<'a>(v: Option<&'a PerlData>, path: &[&str]) -> Option<&'a PerlData> {
    let mut cur = v?;
    for p in path {
        cur = cur.as_object()?.get(*p)?;
    }
    Some(cur)
}

/// `sort keys %$h` (Perl string order).
fn sorted_keys(v: Option<&PerlData>) -> Vec<&str> {
    let mut ks: Vec<&str> = v
        .and_then(J::as_object)
        .map(|o| o.keys().map(String::as_str).collect())
        .unwrap_or_default();
    ks.sort_unstable();
    ks
}

/// An option value interpolated into a string.
fn opt_string(v: Option<&OptValue>) -> Option<String> {
    match v {
        None | Some(OptValue::Undef) => None,
        Some(OptValue::Str(s)) => Some(s.clone()),
        Some(other) => Some(other.to_json().to_string()),
    }
}

/// `map_boolean($opt, $val, 'tostring')` (Utils.pm:1262).
fn map_boolean_tostring(ctx: &Ctx, opt: &str, val: Option<&str>) -> Option<String> {
    if ctx.cfg.blx.registry.datatype(opt) != Some("boolean") {
        return val.map(str::to_string);
    }
    map_boolean_tostring_raw(val)
}

/// Join option items with `,` where `None` items are placeholders that
/// were removed with `s/<BDS>…<\/BDS>,?//`: each takes its following comma,
/// so a removed *last* item leaves a trailing comma after the kept ones.
fn join_removable(items: &[Option<String>]) -> String {
    let kept: Vec<&str> = items.iter().flatten().map(String::as_str).collect();
    let mut s = kept.join(",");
    if !kept.is_empty() && matches!(items.last(), Some(None)) {
        s.push(',');
    }
    s
}

/// The "dangling comma" cleanup `s/,(?:\n\s+)?\}\}/}}/gxms`
/// (DataList.pm:2222), applied to a whole instantiated entry (BB-015).
pub fn dangling_comma_cleanup(s: &str) -> String {
    let b: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == ',' {
            // `\n\s+` then `}}`
            if b.get(i + 1) == Some(&'\n') {
                let mut k = i + 2;
                while k < b.len() && is_perl_space(b[k]) {
                    k += 1;
                }
                if k > i + 2 && b.get(k) == Some(&'}') && b.get(k + 1) == Some(&'}') {
                    out.push_str("}}");
                    i = k + 2;
                    continue;
                }
            }
            if b.get(i + 1) == Some(&'}') && b.get(i + 2) == Some(&'}') {
                out.push_str("}}");
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// `join_name` (Utils.pm:1104): whitespace → `\bibnamedelimb `, unescaped
/// `~` → `\bibnamedelima `, and either after a `.` → `\bibnamedelimi`.
pub fn join_name(s: &str) -> String {
    // s/(?<!\\\S)\s+/\\bibnamedelimb /gxms
    let c: Vec<char> = s.chars().collect();
    let mut a = String::with_capacity(s.len() + 16);
    let mut i = 0;
    while i < c.len() {
        if is_perl_space(c[i]) {
            let blocked = i >= 2 && c[i - 2] == '\\' && !is_perl_space(c[i - 1]);
            if !blocked {
                while i < c.len() && is_perl_space(c[i]) {
                    i += 1;
                }
                a.push_str("\\bibnamedelimb ");
                continue;
            }
        }
        a.push(c[i]);
        i += 1;
    }
    // s/(?<!\\)~/\\bibnamedelima /gxms
    let c: Vec<char> = a.chars().collect();
    let mut b = String::with_capacity(a.len() + 16);
    for (i, ch) in c.iter().enumerate() {
        if *ch == '~' && !(i >= 1 && c[i - 1] == '\\') {
            b.push_str("\\bibnamedelima ");
        } else {
            b.push(*ch);
        }
    }
    // s/(?<=\.)\\bibnamedelim[ab]/\\bibnamedelimi/gxms
    let mut out = String::with_capacity(b.len());
    let mut rest = b.as_str();
    let mut prev_dot = false;
    while !rest.is_empty() {
        if prev_dot && (rest.starts_with("\\bibnamedelima") || rest.starts_with("\\bibnamedelimb"))
        {
            out.push_str("\\bibnamedelimi");
            rest = &rest["\\bibnamedelima".len()..];
            prev_dot = false;
            continue;
        }
        let ch = rest.chars().next().unwrap_or_default();
        out.push(ch);
        prev_dot = ch == '.';
        rest = &rest[ch.len_utf8()..];
    }
    out
}

/// `rangelen` (Utils.pm:1638): the number of pages in a range field, or
/// `-1` if it can't be computed. `zero_digit_quirk` reproduces BB-016.
pub fn rangelen(ranges: &[Vec<Option<String>>], zero_digit_quirk: bool) -> String {
    let is_num = |s: &str| {
        !s.is_empty()
            && s.chars().all(|c| {
                Prop::Nd.contains(c) || Prop::Nl.contains(c) || "iIvVxXlLcCdDmM".contains(c)
            })
    };
    let mut rl = PerlNum::Int(0);
    for f in ranges {
        let m = f.first().cloned().flatten();
        let n = f.get(1).cloned().flatten();
        let Some(m) = m.filter(|m| perl_truthy(m) && is_num(m)) else {
            return "-1".into();
        };
        let m = nfkd(&m).into_owned();
        match n {
            Some(n) if perl_truthy(&n) && is_num(&n) => {
                let n = nfkd(&n).into_owned();
                let roman = |x: String| -> String {
                    if perl_compat::roman::isroman(&x) {
                        perl_compat::roman::roman2int(&x)
                            .map(|v| v.to_string())
                            .unwrap_or_default()
                    } else {
                        x
                    }
                };
                let m = roman(m);
                let mut n = roman(n);
                if !(looks_like_number(&n) && looks_like_number(&m)) {
                    return "-1".into();
                }
                let (mn, nn) = (PerlNum::parse(&m), PerlNum::parse(&n));
                if nn.num_cmp(mn) == Some(std::cmp::Ordering::Less) {
                    let mv: Vec<char> = m.chars().rev().collect();
                    let mut nv: Vec<Option<char>> = n.chars().rev().map(Some).collect();
                    for (i, mc) in mv.iter().enumerate() {
                        if i >= nv.len() {
                            nv.resize(i + 1, None);
                        }
                        let present = match nv[i] {
                            None => false,
                            Some(c) if zero_digit_quirk => perl_truthy(&c.to_string()),
                            Some(_) => true,
                        };
                        if present {
                            continue;
                        }
                        nv[i] = Some(*mc);
                    }
                    n = nv.iter().rev().flatten().collect();
                }
                let len = PerlNum::parse(&n)
                    .minus(PerlNum::parse(&m))
                    .plus(PerlNum::Int(1));
                rl = rl.plus(len);
            }
            Some(_) => return "-1".into(),
            None => rl = rl.plus(PerlNum::Int(1)),
        }
    }
    rl.to_string()
}

// ---------------------------------------------------------------------------
// Entry templates (set_output_entry)
// ---------------------------------------------------------------------------

/// A list-dependent line (`      <BDS>…</BDS>\n` in biber).
#[derive(Clone, Debug, PartialEq, Eq)]
enum Slot {
    EntrySet,
    LabelAlpha,
    ExtraAlpha,
    SortInit,
    SortInitHash,
    LabelPrefix,
    /// `\strng{<name>}{…}` from the list's entry fields (`namehash`,
    /// `authorfullhash`, …).
    Hash(String),
    ExtraName,
    ExtraDate,
    ExtraTitle,
    ExtraTitleYear,
    /// `\true{<name>}` from the list's entry fields (`singletitle`,
    /// `uniquetitle`, …).
    True(&'static str),
}

/// One name of a `\name` block (`name_to_bbl`, Name.pm:421).
#[derive(Clone, Debug)]
struct NameT {
    nid: NameId,
    /// `$un ne 'false'`: uniquename placeholders present.
    un: bool,
    /// Per-name options (`no=value`).
    opts: Vec<String>,
    /// `(namepart, value, initials)` of the non-empty parts.
    parts: Vec<(String, String, String)>,
}

/// A `\name` block.
#[derive(Clone, Debug)]
struct NameBlockT {
    field: String,
    nlid: NameListId,
    total: usize,
    /// The `UL-<id>` placeholder is present (labelname, `uniquelist` on).
    ul: bool,
    /// Per-namelist options.
    nlopts: Vec<String>,
    names: Vec<NameT>,
}

#[derive(Clone, Debug)]
enum Item {
    Text(String),
    Slot(Slot),
    Names(NameBlockT),
}

/// The per-section part of an entry's output (biber's placeholder string).
#[derive(Clone, Debug)]
struct EntryTemplate {
    items: Vec<Item>,
    /// For each name id in the entry's output name lists: the name list of
    /// its first occurrence (in `namelists` order), whose uniquename data
    /// the global substitution uses.
    un_src: HashMap<NameId, NameListId>,
    /// Same for the per-name hash (`namelistsall` order).
    hash_src: HashMap<NameId, NameListId>,
}

/// Mutable writer state that lives across sections.
struct Writer {
    /// `$dmh->{datefields}`, which the date-meta loop mutates in place
    /// (BB-028).
    datefields: Vec<String>,
    /// The current section's shared list arrays: `(citekey, field)` →
    /// group, and each group's live value (B21).
    shared: HashMap<(String, String), usize>,
    shared_vals: Vec<Option<Vec<String>>>,
}

impl Writer {
    fn start_section(&mut self, st: &SectionState) {
        self.shared.clear();
        self.shared_vals.clear();
        for (i, g) in st.shared_lists.iter().flatten().enumerate() {
            for m in g {
                self.shared.insert(m.clone(), i);
            }
            self.shared_vals.push(None);
        }
    }

    /// Replace the live value of a shared list.
    fn set_live(&mut self, key: &str, field: &str, v: Vec<String>) {
        if let Some(&g) = self.shared.get(&(key.to_string(), field.to_string())) {
            self.shared_vals[g] = Some(v);
        }
    }

    /// The list to output for `key`'s `field` (value `orig` in the
    /// state), after biber's `pop` of a trailing `others` (bbl.pm:335,
    /// 567), which changes the live array, shared with other entries when
    /// the field was inherited (05-B21). Returns whether it popped.
    fn pop_others(
        &mut self,
        key: &str,
        field: &str,
        orig: &[String],
        others: &str,
    ) -> (bool, Vec<String>) {
        let is_others = |l: &[String]| l.last().map(|x| lc(x)).unwrap_or_default() == others;
        match self.shared.get(&(key.to_string(), field.to_string())) {
            Some(&g) => {
                let live = self.shared_vals[g].get_or_insert_with(|| orig.to_vec());
                let popped = is_others(live);
                if popped {
                    live.pop();
                }
                (popped, live.clone())
            }
            None => {
                let popped = is_others(orig);
                let n = orig.len() - usize::from(popped);
                (popped, orig[..n].to_vec())
            }
        }
    }
}

fn add_names(
    arena: &NameArena,
    fields: &[String],
    e: &Entry,
    map: &mut HashMap<NameId, NameListId>,
) {
    for f in fields {
        if let Some(Value::Names(id)) = e.get_field(f) {
            for n in &arena.list(*id).names {
                map.entry(*n).or_insert(*id);
            }
        }
    }
}

/// `filter_entry_options` (Utils.pm:1120).
fn filter_entry_options(ctx: &Ctx, secnum: &str, key: &str) -> Vec<String> {
    let mut out = Vec::new();
    for opt in ctx.cfg.blx.entry_options(secnum, key) {
        let val = opt_string(ctx.cfg.blx.get(Some(secnum), opt, None, Some(key)));
        let val = map_boolean_tostring(ctx, opt, val.as_deref());
        let cfopt = ctx
            .cfg
            .blx
            .registry
            .decl(opt, Scope::Entry)
            .is_some_and(|d| perl_truthy(&d.output));
        if cfopt {
            match val {
                Some(v) if perl_truthy(&v) => out.push(format!("{opt}={v}")),
                _ => out.push(opt.to_string()),
            }
        }
    }
    out
}

/// `getblxoption(undef, $opt, $entrytype, $key)` as a Perl boolean.
fn blx_undef_truthy(ctx: &Ctx, secnum: &str, opt: &str, et: &str, key: &str) -> bool {
    match ctx
        .cfg
        .blx
        .get_undef_secnum(ctx.cfg.quirks, secnum, opt, Some(et), Some(key))
    {
        Some(OptValue::Str(s)) => perl_truthy(s),
        Some(OptValue::Undef) | None => false,
        Some(_) => true,
    }
}

/// `name_to_bbl` (Name.pm:421) without the uniquename/hash values.
fn name_template(ctx: &Ctx, dm: &DataModel, n: &Name, un: bool) -> NameT {
    let mut parts = Vec::new();
    for np in dm.get_constant_value("nameparts") {
        let part = n.nameparts.get(&np);
        let s = part.and_then(|p| p.string.clone()).unwrap_or_default();
        let mut npc = String::new();
        let mut npci = String::new();
        if perl_truthy(&s) {
            npc = if n
                .strip
                .get(&np)
                .and_then(|v| v.as_deref())
                .is_some_and(perl_truthy)
            {
                format!("{{{s}}}")
            } else {
                join_name(&s)
            };
            let inits = part.and_then(|p| p.initial.clone()).unwrap_or_default();
            let i = inits.join("\\bibinitperiod\\bibinitdelim ") + "\\bibinitperiod";
            npci = i
                .chars()
                .map(|c| {
                    if Prop::Pd.contains(c) {
                        "\\bibinithyphendelim ".to_string()
                    } else {
                        c.to_string()
                    }
                })
                .collect();
        }
        if perl_truthy(&npc) {
            parts.push((np, npc, npci));
        }
    }
    let mut opts = Vec::new();
    for no in ctx.cfg.blx.registry.options_in_scope(Scope::Name) {
        let Some(v) = n.slots.get(no).filter(|v| !v.is_null()) else {
            continue;
        };
        let out = ctx
            .cfg
            .blx
            .registry
            .decl(no, Scope::Name)
            .is_some_and(|d| perl_truthy(&d.output));
        if out {
            let v = pd_string(Some(v));
            let v = map_boolean_tostring(ctx, no, Some(&v)).unwrap_or_default();
            opts.push(format!("{no}={v}"));
        }
    }
    NameT {
        nid: n.id,
        un,
        opts,
        parts,
    }
}

/// `_printfield` (bbl.pm:105).
fn printfield(ctx: &Ctx, dm: &DataModel, e: &Entry, field: &str, s: &str) -> String {
    let outfield = dm.get_outcase(field).unwrap_or(field);
    if s.is_empty() && !dm.field_is_nullok(field) {
        return String::new();
    }
    let mut s = s.to_string();
    let lcf = lc(field);
    let field_type = if lcf == "crossref" || lcf == "xref" {
        "strng"
    } else {
        "field"
    };
    // Astronomical years are output as absolute values (`^(.*)(?!end)year$`).
    if field.ends_with("year")
        && let Some(y) = e.get_field(field).filter(|v| val_truthy(Some(v)))
    {
        let y = val_string(Some(y));
        if looks_like_number(&y) {
            s = PerlNum::parse(&y).abs().to_string();
        }
    }
    // Auto-escape TeX specials in non-BibTeX data.
    if field_type != "strng" && e.get_str("datatype") != Some("bibtex") {
        let c: Vec<char> = s.chars().collect();
        let mut o = String::with_capacity(s.len());
        for (i, ch) in c.iter().enumerate() {
            if matches!(ch, '#' | '&' | '%') && !(i >= 1 && c[i - 1] == '\\') {
                o.push('\\');
            }
            o.push(*ch);
        }
        s = o;
    }
    // --wraplines: Text::Wrap with `$columns` = the option value.
    let cols = ctx
        .cfg
        .biber
        .get_str("wraplines")
        .filter(|w| perl_truthy(w))
        .map(|w| PerlNum::parse(w).as_f64() as i64);
    if let Some(cols) = cols {
        // 16 is the length of '      \field{}{}' or '      \strng{}{}'
        let len = 16 + gcstring::length(outfield) as i64 + gcstring::length(&s) as i64;
        if len > 2 * cols {
            return format!(
                "      \\{field_type}{{{outfield}}}{{%\n{}%\n      }}\n",
                textwrap::wrap(cols, "      ", "      ", &s)
            );
        } else if len > cols {
            return textwrap::wrap(
                cols,
                "      ",
                "      ",
                &format!("\\{field_type}{{{outfield}}}{{{s}}}"),
            ) + "\n";
        }
    }
    format!("      \\{field_type}{{{outfield}}}{{{s}}}\n")
}

fn uri_string(ctx: &mut Ctx, s: &str) -> Result<String, Fatal> {
    match perl_compat::uri::uri_as_string(&nfc(s)) {
        Ok(u) => Ok(u),
        Err(e) => {
            biber_error!(ctx.log, "{e}")?;
            Ok(String::new())
        }
    }
}

/// `set_output_entry` (bbl.pm:196): the template of one entry, or `None`
/// for entry types the data model skips.
fn set_output_entry(
    ctx: &mut Ctx,
    w: &mut Writer,
    st: &SectionState,
    key: &str,
) -> Result<Option<EntryTemplate>, Fatal> {
    let dm = ctx.dm;
    let section = &st.section;
    let secnum = section.number.as_str();
    let arena = &section.names;
    let Some(be) = section.bibentry(key) else {
        biber_error!(ctx.log, "Cannot find entry with key '{key}' to output")?;
        return Ok(None);
    };
    let bee = be.entrytype().to_string();
    let outtype = dm.get_outcase(&bee).unwrap_or(&bee).to_string();
    let dmh = &dm.helpers;
    let blx = &ctx.cfg.blx;
    let mut un =
        opt_string(blx.get(Some(secnum), "uniquename", Some(&bee), Some(key))).unwrap_or_default();
    let mut ul =
        opt_string(blx.get(Some(secnum), "uniquelist", Some(&bee), Some(key))).unwrap_or_default();
    let lni: Option<String> = be
        .slots
        .get("labelnameinfo")
        .filter(|v| !v.is_null())
        .map(|v| pd_string(Some(v)));
    let lnl = lni
        .as_deref()
        .and_then(|f| be.get_field(f))
        .and_then(Value::as_names)
        .map(|id| arena.list(id));
    if let Some(nl) = lnl {
        // Per-namelist uniquelist / uniquename.
        if pd_truthy(nl.slots.get("uniquelist")) {
            ul = pd_string(nl.slots.get("uniquelist"));
        }
        if pd_truthy(nl.slots.get("uniquename")) {
            un = pd_string(nl.slots.get("uniquename"));
        }
    }
    if dm.entrytype_is_skipout(&bee) {
        return Ok(None);
    }

    let mut items = Vec::new();
    let mut text = String::new();
    let flush = |text: &mut String, items: &mut Vec<Item>| {
        if !text.is_empty() {
            items.push(Item::Text(std::mem::take(text)));
        }
    };
    let slot = |text: &mut String, items: &mut Vec<Item>, s: Slot| {
        if !text.is_empty() {
            items.push(Item::Text(std::mem::take(text)));
        }
        items.push(Item::Slot(s));
    };

    let citecount = pd_path(section.slots.get("citecount"), &[key]);
    let kc = if pd_truthy(citecount) {
        pd_string(citecount)
    } else {
        "-1".into()
    };
    let kc = if PerlNum::parse(&kc).num_cmp(PerlNum::Int(-1)) == Some(std::cmp::Ordering::Equal) {
        String::new()
    } else {
        kc
    };
    text.push_str(&format!(
        "    \\entry{{{key}}}{{{outtype}}}{{{}}}{{{kc}}}\n",
        filter_entry_options(ctx, secnum, key).join(",")
    ));

    let labelalpha = blx_undef_truthy(ctx, secnum, "labelalpha", &bee, key);
    if bee == "set" {
        slot(&mut text, &mut items, Slot::EntrySet);
        if labelalpha {
            slot(&mut text, &mut items, Slot::LabelAlpha);
            slot(&mut text, &mut items, Slot::ExtraAlpha);
        }
        slot(&mut text, &mut items, Slot::SortInit);
        slot(&mut text, &mut items, Slot::SortInitHash);
        slot(&mut text, &mut items, Slot::LabelPrefix);
        for f in ["label", "annotation", "shorthand"] {
            if val_truthy(be.get_field(f)) {
                text.push_str(&format!(
                    "      \\field{{{f}}}{{{}}}\n",
                    val_string(be.get_field(f))
                ));
            }
        }
        if val_truthy(be.get_field("keywords")) {
            text.push_str(&format!(
                "      \\keyw{{{}}}\n",
                val_string(be.get_field("keywords"))
            ));
        }
        text.push_str("    \\endentry\n");
        flush(&mut text, &mut items);
        return Ok(Some(EntryTemplate {
            items,
            un_src: HashMap::new(),
            hash_src: HashMap::new(),
        }));
    }
    if let Some(es) = be.get_field("entryset").filter(|v| val_truthy(Some(v))) {
        text.push_str(&format!("      \\inset{{{}}}\n", val_string(Some(es))));
    }

    // Names
    for namefield in &dmh.namelists {
        let Some(Value::Names(nlid)) = be.get_field(namefield) else {
            continue;
        };
        let nf = arena.list(*nlid);
        let is_lni = lni.as_deref() == Some(namefield.as_str());
        if pd_truthy(nf.slots.get("morenames")) {
            text.push_str(&format!("      \\true{{more{namefield}}}\n"));
            if is_lni {
                text.push_str("      \\true{morelabelname}\n");
            }
        }
        // Per-name uniquename if this is labelname (and it leaks to the
        // following name lists).
        if is_lni && nf.slots.get("uniquename").is_some_and(|v| !v.is_null()) {
            un = pd_string(nf.slots.get("uniquename"));
        }
        let mut block = NameBlockT {
            field: namefield.clone(),
            nlid: *nlid,
            total: nf.names.len(),
            ul: false,
            nlopts: Vec::new(),
            names: Vec::new(),
        };
        if is_lni {
            block.ul = ul != "false";
            // Per-namelist options (DC-013: declaration order).
            for nlo in ctx.cfg.blx.registry.options_in_scope(Scope::NameList) {
                let Some(v) = nf.slots.get(nlo).filter(|v| !v.is_null()) else {
                    continue;
                };
                let out = ctx
                    .cfg
                    .blx
                    .registry
                    .decl(nlo, Scope::NameList)
                    .is_some_and(|d| perl_truthy(&d.output));
                if out {
                    let v = pd_string(Some(v));
                    let v = map_boolean_tostring(ctx, nlo, Some(&v)).unwrap_or_default();
                    block.nlopts.push(format!("{nlo}={v}"));
                }
            }
        }
        for n in arena.names_of(*nlid) {
            block.names.push(name_template(ctx, dm, n, un != "false"));
        }
        flush(&mut text, &mut items);
        items.push(Item::Names(block));
    }

    // Lists
    let others = ctx
        .cfg
        .biber
        .get_str("others_string")
        .unwrap_or("others")
        .to_string();
    for listfield in &dmh.lists {
        let Some(Value::List(lf)) = be.get_field(listfield) else {
            continue;
        };
        let (popped, lf) = w.pop_others(key, listfield, lf, &others);
        if popped {
            text.push_str(&format!("      \\true{{more{listfield}}}\n"));
        }
        text.push_str(&format!("      \\list{{{listfield}}}{{{}}}{{%\n", lf.len()));
        for f in &lf {
            text.push_str(&format!("        {{{f}}}%\n"));
        }
        text.push_str("      }\n");
    }

    for h in ["namehash", "fullhash", "fullhashraw", "bibnamehash"] {
        slot(&mut text, &mut items, Slot::Hash(h.into()));
    }
    for namefield in &dmh.namelists {
        if !val_truthy(be.get_field(namefield)) {
            continue;
        }
        for h in ["bibnamehash", "namehash", "fullhash", "fullhashraw"] {
            slot(&mut text, &mut items, Slot::Hash(format!("{namefield}{h}")));
        }
    }
    let lni_t = lni.as_deref().is_some_and(perl_truthy);
    if lni_t {
        slot(&mut text, &mut items, Slot::ExtraName);
    }
    if labelalpha {
        slot(&mut text, &mut items, Slot::LabelAlpha);
    }
    slot(&mut text, &mut items, Slot::SortInit);
    slot(&mut text, &mut items, Slot::SortInitHash);
    if blx_undef_truthy(ctx, secnum, "labeldateparts", &bee, key) {
        slot(&mut text, &mut items, Slot::ExtraDate);
        if val_truthy(be.get_field("extradatescope")) {
            text.push_str(&format!(
                "      \\field{{extradatescope}}{{{}}}\n",
                val_string(be.get_field("extradatescope"))
            ));
        }
        if be.field_exists("labeldatesource") {
            text.push_str(&format!(
                "      \\field{{labeldatesource}}{{{}}}\n",
                val_string(be.get_field("labeldatesource"))
            ));
        }
    }
    if !val_truthy(be.get_field("shorthand")) {
        slot(&mut text, &mut items, Slot::LabelPrefix);
    }
    if blx_undef_truthy(ctx, secnum, "labeltitle", &bee, key) {
        slot(&mut text, &mut items, Slot::ExtraTitle);
    }
    if blx_undef_truthy(ctx, secnum, "labeltitleyear", &bee, key) {
        slot(&mut text, &mut items, Slot::ExtraTitleYear);
    }
    if labelalpha {
        slot(&mut text, &mut items, Slot::ExtraAlpha);
    }
    if be.get_field("crossrefsource").is_some() {
        text.push_str("      \\true{crossrefsource}\n");
    }
    if be.get_field("xrefsource").is_some() {
        text.push_str("      \\true{xrefsource}\n");
    }
    for t in [
        "singletitle",
        "uniquetitle",
        "uniquebaretitle",
        "uniquework",
        "uniqueprimaryauthor",
    ] {
        slot(&mut text, &mut items, Slot::True(t));
    }
    if lni_t {
        text.push_str(&format!(
            "      \\field{{labelnamesource}}{{{}}}\n",
            lni.as_deref().unwrap_or("")
        ));
    }
    let lti = be.slots.get("labeltitleinfo");
    if pd_truthy(lti) {
        text.push_str(&format!(
            "      \\field{{labeltitlesource}}{{{}}}\n",
            pd_string(lti)
        ));
    }
    if val_truthy(be.get_field("clonesourcekey")) {
        text.push_str(&format!(
            "      \\field{{clonesourcekey}}{{{}}}\n",
            val_string(be.get_field("clonesourcekey"))
        ));
    }

    // Plain fields
    for field in &dmh.fields {
        let val = be.get_field(field);
        let has = !val_string(val).is_empty() && val.is_some();
        if has || (dm.field_is_nullok(field) && be.field_exists(field)) {
            if field == "crossref" || field == "xref" {
                let target = val_string(be.get_field(field));
                if !has_citekey(st, &target) {
                    continue;
                }
            }
            text.push_str(&printfield(ctx, dm, be, field, &val_string(val)));
        }
    }

    // Date meta-information. BB-028: the loop variable aliases the shared
    // helper list, so `s/date$//` strips it in place for later entries.
    let stems: Vec<String> = if ctx.cfg.quirks.on(Quirk::DatefieldHelperMutation) {
        for d in &mut w.datefields {
            if let Some(s) = d.strip_suffix("date") {
                *d = s.to_string();
            }
        }
        w.datefields.clone()
    } else {
        dmh.datefields
            .iter()
            .map(|d| d.strip_suffix("date").unwrap_or(d).to_string())
            .collect()
    };
    for d in &stems {
        let unspec = be.get_field(&format!("{d}dateunspecified"));
        if val_truthy(unspec) {
            text.push_str(&format!(
                "      \\field{{{d}dateunspecified}}{{{}}}\n",
                val_string(unspec)
            ));
        }
        for (src, out) in [
            ("datejulian", "datejulian"),
            ("enddatejulian", "enddatejulian"),
            ("dateapproximate", "datecirca"),
            ("enddateapproximate", "enddatecirca"),
            ("dateuncertain", "dateuncertain"),
            ("enddateuncertain", "enddateuncertain"),
            ("dateunknown", "dateunknown"),
            ("enddateunknown", "enddateunknown"),
        ] {
            if val_truthy(be.get_field(&format!("{d}{src}"))) {
                text.push_str(&format!("      \\true{{{d}{out}}}\n"));
            }
        }
        if be.field_exists(&format!("{d}endyear")) {
            let era = be.get_field(&format!("{d}endera"));
            if val_truthy(era) {
                text.push_str(&format!(
                    "      \\field{{{d}enddateera}}{{{}}}\n",
                    val_string(era)
                ));
            }
        }
        if be.field_exists(&format!("{d}year")) {
            if !val_truthy(be.get_field(&format!("{d}datesplit"))) {
                continue;
            }
            let era = be.get_field(&format!("{d}era"));
            if val_truthy(era) {
                text.push_str(&format!(
                    "      \\field{{{d}dateera}}{{{}}}\n",
                    val_string(era)
                ));
            }
        }
    }

    // XSV fields
    for field in &dmh.xsv {
        if field == "keywords" {
            continue;
        }
        let f = be.get_field(field);
        if val_truthy(f) {
            text.push_str(&printfield(ctx, dm, be, field, &val_string(f)));
        }
    }

    if val_truthy(be.get_field("nocite")) {
        text.push_str("      \\true{nocite}\n");
    }

    // Ranges
    let zero_quirk = ctx.cfg.quirks.on(Quirk::RangelenZeroDigit);
    for rfield in &dmh.ranges {
        let Some(Value::Ranges(rf)) = be.get_field(rfield) else {
            continue;
        };
        let pr: Vec<String> = rf
            .iter()
            .map(|f| {
                let start = f.first().cloned().flatten().unwrap_or_default();
                match f.get(1).cloned().flatten() {
                    Some(end) => format!(
                        "{start}\\bibrangedash{}",
                        if perl_truthy(&end) {
                            format!(" {end}")
                        } else {
                            String::new()
                        }
                    ),
                    None => start,
                }
            })
            .collect();
        text.push_str(&format!(
            "      \\field{{{rfield}}}{{{}}}\n",
            pr.join("\\bibrangessep ")
        ));
        text.push_str(&format!(
            "      \\range{{{rfield}}}{{{}}}\n",
            rangelen(rf, zero_quirk)
        ));
    }

    // Verbatim fields
    for vfield in &dmh.vfields {
        let v = be.get_field(vfield);
        if !val_truthy(v) {
            continue;
        }
        let mut vf = val_string(v);
        if dm.get_datatype(vfield) == Some("uri") {
            text.push_str(&format!("      \\verb{{{vfield}raw}}\n"));
            text.push_str(&format!("      \\verb {vf}\n      \\endverb\n"));
            vf = uri_string(ctx, &vf)?;
        }
        text.push_str(&format!("      \\verb{{{vfield}}}\n"));
        text.push_str(&format!("      \\verb {vf}\n      \\endverb\n"));
    }

    // Verbatim lists
    for vlist in &dmh.vlists {
        let Some(Value::List(vlf)) = be.get_field(vlist) else {
            continue;
        };
        let (popped, vlf) = w.pop_others(key, vlist, vlf, &others);
        if popped {
            text.push_str(&format!("      \\true{{more{vlist}}}\n"));
        }
        let total = vlf.len();
        let is_uri = dm.get_datatype(vlist) == Some("uri");
        if is_uri {
            text.push_str(&format!("      \\lverb{{{vlist}raw}}{{{total}}}\n"));
            for f in &vlf {
                text.push_str(&format!("      \\lverb {f}\n"));
            }
            text.push_str("      \\endlverb\n");
        }
        text.push_str(&format!("      \\lverb{{{vlist}}}{{{total}}}\n"));
        let mut enc = Vec::with_capacity(vlf.len());
        for f in &vlf {
            let f = if is_uri {
                uri_string(ctx, f)?
            } else {
                f.clone()
            };
            text.push_str(&format!("      \\lverb {f}\n"));
            enc.push(f);
        }
        text.push_str("      \\endlverb\n");
        // The loop variable aliases the live array: URIs stay encoded.
        if is_uri {
            w.set_live(key, vlist, enc);
        }
    }

    if val_truthy(be.get_field("keywords")) {
        text.push_str(&format!(
            "      \\keyw{{{}}}\n",
            val_string(be.get_field("keywords"))
        ));
    }

    // Annotations (Biber::Annotation, sorted at every level)
    let ann = pd_path(st.annotations.as_ref(), &[key]);
    let nm = |n: &str| {
        if perl_truthy(n) {
            n.to_string()
        } else {
            "default".into()
        }
    };
    let field_ann = pd_path(ann, &["field"]);
    for f in sorted_keys(field_ann) {
        let fa = pd_path(field_ann, &[f]);
        for n in sorted_keys(fa) {
            let a = pd_path(fa, &[&nm(n)]);
            text.push_str(&format!(
                "      \\annotation{{field}}{{{f}}}{{{n}}}{{}}{{}}{{{}}}{{{}}}\n",
                pd_string(pd_path(a, &["literal"])),
                pd_string(pd_path(a, &["value"]))
            ));
        }
    }
    let item_ann = pd_path(ann, &["item"]);
    for f in sorted_keys(item_ann) {
        let fa = pd_path(item_ann, &[f]);
        for n in sorted_keys(fa) {
            let na = pd_path(fa, &[&nm(n)]);
            for c in sorted_keys(na) {
                let a = pd_path(na, &[c]);
                text.push_str(&format!(
                    "      \\annotation{{item}}{{{f}}}{{{n}}}{{{c}}}{{}}{{{}}}{{{}}}\n",
                    pd_string(pd_path(a, &["literal"])),
                    pd_string(pd_path(a, &["value"]))
                ));
            }
        }
    }
    let part_ann = pd_path(ann, &["part"]);
    for f in sorted_keys(part_ann) {
        let fa = pd_path(part_ann, &[f]);
        for n in sorted_keys(fa) {
            let na = pd_path(fa, &[&nm(n)]);
            for c in sorted_keys(na) {
                let ca = pd_path(na, &[c]);
                for p in sorted_keys(ca) {
                    let a = pd_path(ca, &[p]);
                    text.push_str(&format!(
                        "      \\annotation{{part}}{{{f}}}{{{n}}}{{{c}}}{{{p}}}{{{}}}{{{}}}\n",
                        pd_string(pd_path(a, &["literal"])),
                        pd_string(pd_path(a, &["value"]))
                    ));
                }
            }
        }
    }

    if let Some(Value::List(ws)) = be.get_field("warnings") {
        for w in ws {
            text.push_str(&format!("      \\warn{{\\item {w}}}\n"));
        }
    }
    text.push_str("    \\endentry\n");
    flush(&mut text, &mut items);

    let mut un_src = HashMap::new();
    add_names(arena, &dmh.namelists, be, &mut un_src);
    let mut hash_src = HashMap::new();
    add_names(arena, &dmh.namelistsall, be, &mut hash_src);
    Ok(Some(EntryTemplate {
        items,
        un_src,
        hash_src,
    }))
}

/// `Biber::Section::has_citekey` (aliases resolved).
fn has_citekey(st: &SectionState, key: &str) -> bool {
    let s = &st.section;
    let real = s
        .citekey_alias
        .get(key)
        .filter(|k| perl_truthy(k))
        .map(String::as_str)
        .unwrap_or(key);
    s.citekeys_h.contains(real)
}

// ---------------------------------------------------------------------------
// Per-list instantiation (DataList::instantiate_entry)
// ---------------------------------------------------------------------------

/// Per-list lookups (`Biber::DataList` getters).
struct ListView<'a> {
    dl: &'a DataList,
    state: Option<&'a PerlData>,
    sortinithash: &'a dyn Fn(&str) -> String,
}

impl<'a> ListView<'a> {
    fn entryfield(&self, key: &str, f: &str) -> Option<&'a PerlData> {
        pd_path(self.state, &["fields", key, f])
    }
    fn data(&self, slot: &str, key: &str) -> Option<&'a PerlData> {
        pd_path(self.state, &[slot, key])
    }
    fn nld(&self, nlid: NameListId) -> Option<&'a PerlData> {
        pd_path(self.state, &["namelistdata", &nlid.key()])
    }
    fn nd(&self, nlid: NameListId, nid: NameId) -> Option<&'a PerlData> {
        pd_path(self.nld(nlid), &[&nid.key()])
    }
    /// `get_unsummary` (DataList.pm:408).
    fn unsummary(&self, nlid: NameListId, nid: NameId) -> Option<&'static str> {
        let un = pd_path(self.nd(nlid, nid), &["un"])?;
        if un.is_null() {
            return None;
        }
        let a = un.as_array();
        let get = |i: usize| a.and_then(|a| a.get(i)).map(|v| pd_string(Some(v)));
        let (p, l) = (get(0).unwrap_or_default(), get(1).unwrap_or_default());
        Some(if l == "none" || p == "base" {
            "0"
        } else if l == "init" {
            "1"
        } else if l == "full" || l == "fullonly" {
            "2"
        } else {
            "0"
        })
    }
    fn sortinit(&self, key: &str) -> Option<String> {
        let v = pd_path(self.dl.slots.get("sortinitdata"), &[key, "init"])?;
        if v.is_null() {
            None
        } else {
            Some(pd_string(Some(v)))
        }
    }
}

fn instantiate_entry(lv: &ListView, t: &EntryTemplate, key: &str) -> String {
    let mut s = String::new();
    for item in &t.items {
        match item {
            Item::Text(x) => s.push_str(x),
            Item::Slot(slot) => {
                let line = match slot {
                    Slot::EntrySet => match lv.entryfield(key, "entryset") {
                        Some(J::Array(a)) => Some(format!(
                            "\\set{{{}}}",
                            a.iter()
                                .map(|v| pd_string(Some(v)))
                                .collect::<Vec<_>>()
                                .join(",")
                        )),
                        v if pd_truthy(v) => Some(format!("\\set{{{}}}", pd_string(v))),
                        _ => None,
                    },
                    Slot::True(f) => {
                        pd_truthy(lv.entryfield(key, f)).then(|| format!("\\true{{{f}}}"))
                    }
                    Slot::Hash(f) => {
                        let v = lv.entryfield(key, f);
                        pd_truthy(v).then(|| format!("\\strng{{{f}}}{{{}}}", pd_string(v)))
                    }
                    Slot::LabelAlpha
                    | Slot::ExtraAlpha
                    | Slot::ExtraName
                    | Slot::ExtraDate
                    | Slot::ExtraTitle
                    | Slot::ExtraTitleYear => {
                        let (data, name) = match slot {
                            Slot::LabelAlpha => ("labelalphadata", "labelalpha"),
                            Slot::ExtraAlpha => ("extraalphadata", "extraalpha"),
                            Slot::ExtraName => ("extranamedata", "extraname"),
                            Slot::ExtraDate => ("extradatedata", "extradate"),
                            Slot::ExtraTitle => ("extratitledata", "extratitle"),
                            _ => ("extratitleyeardata", "extratitleyear"),
                        };
                        let v = lv.data(data, key);
                        pd_truthy(v).then(|| format!("\\field{{{name}}}{{{}}}", pd_string(v)))
                    }
                    Slot::SortInit => lv
                        .sortinit(key)
                        .map(|si| format!("\\field{{sortinit}}{{{si}}}")),
                    Slot::SortInitHash => lv
                        .sortinit(key)
                        .map(|si| format!("\\field{{sortinithash}}{{{}}}", (lv.sortinithash)(&si))),
                    Slot::LabelPrefix => perl_truthy(&lv.dl.labelprefix)
                        .then(|| format!("\\field{{labelprefix}}{{{}}}", lv.dl.labelprefix)),
                };
                if let Some(l) = line {
                    s.push_str("      ");
                    s.push_str(&l);
                    s.push('\n');
                }
            }
            Item::Names(b) => {
                let ul = if b.ul {
                    let v = pd_path(lv.nld(b.nlid), &["ul"]).filter(|v| !v.is_null());
                    vec![v.map(|v| format!("ul={}", pd_string(Some(v))))]
                } else {
                    vec![]
                };
                let mut nlopts = ul;
                nlopts.extend(b.nlopts.iter().cloned().map(Some));
                s.push_str(&format!(
                    "      \\name{{{}}}{{{}}}{{{}}}{{%\n",
                    b.field,
                    b.total,
                    join_removable(&nlopts)
                ));
                for n in &b.names {
                    let un_nl = t.un_src.get(&n.nid).copied().unwrap_or(b.nlid);
                    let uns = lv.unsummary(un_nl, n.nid);
                    let mut pno: Vec<Option<String>> = Vec::new();
                    if n.un {
                        match uns {
                            Some(u) => {
                                pno.push(Some(format!("un={u}")));
                                let part = pd_string(
                                    pd_path(lv.nd(un_nl, n.nid), &["un"]).and_then(|v| v.get(0)),
                                );
                                pno.push(Some(format!("uniquepart={part}")));
                            }
                            None => {
                                pno.push(None);
                                pno.push(None);
                            }
                        }
                    }
                    pno.extend(n.opts.iter().cloned().map(Some));
                    let h_nl = t.hash_src.get(&n.nid).copied().unwrap_or(b.nlid);
                    let h = pd_path(lv.nd(h_nl, n.nid), &["hash"]);
                    pno.push(pd_truthy(h).then(|| format!("hash={}", pd_string(h))));
                    let mut lines = Vec::new();
                    for (np, v, i) in &n.parts {
                        lines.push(format!("           {np}={{{v}}}"));
                        lines.push(format!("           {np}i={{{i}}}"));
                        if n.un && uns.is_some() {
                            let nd = lv.nd(un_nl, n.nid);
                            let base = pd_path(nd, &["basenamestringparts"])
                                .and_then(J::as_array)
                                .is_some_and(|a| a.iter().any(|x| x.as_str() == Some(np)));
                            if !base {
                                lines.push(format!(
                                    "           {np}un={}",
                                    pd_string(pd_path(nd, &["unparts", np]))
                                ));
                            }
                        }
                    }
                    s.push_str(&format!(
                        "        {{{{{}}}{{%\n{}}}}}%\n",
                        join_removable(&pno),
                        lines.join(",\n")
                    ));
                }
                s.push_str("      }\n");
            }
        }
    }
    s
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

/// What `create_output_section` records for one section.
struct SectionOut<'a> {
    st: &'a SectionState,
    entries: HashMap<String, EntryTemplate>,
}

/// The output lists of a section in `.bbl` order (bbl.pm:689-707).
fn output_lists<'a>(ctx: &Ctx, st: &'a SectionState) -> Vec<&'a DataList> {
    let global = opt_string(ctx.cfg.blx.global("sortingtemplatename")).unwrap_or_default();
    let mut lists: Vec<&DataList> = st.datalists.iter().collect();
    lists.sort_by(|a, b| a.sortingtemplatename.cmp(&b.sortingtemplatename));
    let mut out: Vec<&DataList> = lists
        .into_iter()
        .filter(|l| !(l.sortingtemplatename == global && l.type_ == "entry"))
        .collect();
    out.extend(
        st.datalists
            .iter()
            .filter(|l| l.sortingtemplatename == global && l.type_ == "entry"),
    );
    out
}

/// The `output_encoding` option as given (`UTF-8` after `normalise_utf8`).
pub fn output_encoding_name(ctx: &Ctx) -> String {
    ctx.cfg
        .biber
        .get_str("output_encoding")
        .unwrap_or("UTF-8")
        .to_string()
}

/// The output encoding (`Encode::find_encoding`). biber dies when the name
/// is unknown to Encode (a 0-byte `.bbl` and a Perl error); bibers reports an
/// ERROR instead. `UTF-7`, `gsm0338` and the `MIME-*` encodings are not
/// supported (docs/compat.md DC-034).
pub(crate) fn find_output_encoding(ctx: &mut Ctx) -> Result<Encoding, Fatal> {
    let name = output_encoding_name(ctx);
    match lookup(&name) {
        Lookup::Found(e) => Ok(e),
        Lookup::Unsupported(canon) => {
            biber_error!(
                ctx.log,
                "Output encoding '{name}' ('{canon}') is not supported (docs/compat.md DC-034)"
            )?;
            Ok(Encoding::UTF8)
        }
        Lookup::Unknown => {
            biber_error!(ctx.log, "Unknown output encoding '{name}'")?;
            Ok(Encoding::UTF8)
        }
    }
}

/// The bytes to write for the rendered `.bbl` text: encoded as a Perl
/// `:encoding(<output_encoding>)` layer would (unmappable characters as
/// `\x{XXXX}`; for `UTF-8`, i.e. `utf-8-strict`, noncharacters too).
pub fn encode_output(ctx: &mut Ctx, text: &str) -> Result<Vec<u8>, Fatal> {
    let enc = if output_encoding_name(ctx) == "UTF-8" {
        Encoding::UTF8
    } else {
        find_output_encoding(ctx)?
    };
    Ok(enc.encode_layer(text, ctx.cfg.quirks.on(Quirk::PerlioEncodingSplit)))
}

/// The locale of a sorting template (for the sortinit collator).
fn template_locale(ctx: &Ctx, name: &str) -> Option<String> {
    match ctx.cfg.blx.global("sortingtemplate") {
        Some(OptValue::SortingTemplates(m)) => m.get(name).and_then(|t| t.locale.clone()),
        _ => None,
    }
}

/// Render the whole `.bbl` (header, `\preamble`, one `\refsection` per
/// section with output entries, `\endinput`).
pub fn render(ctx: &mut Ctx, processed: &Processed) -> Result<Option<String>, Fatal> {
    let mut w = Writer {
        datefields: ctx.dm.helpers.datefields.clone(),
        shared: HashMap::new(),
        shared_vals: Vec::new(),
    };
    // create_output_section, per section in processing order.
    let mut sections: Vec<SectionOut> = Vec::new();
    for st in &processed.sections {
        w.start_section(st);
        let mut entries = HashMap::new();
        for k in &st.section.citekeys {
            if let Some(t) = set_output_entry(ctx, &mut w, st, k)? {
                entries.insert(k.clone(), t);
            }
        }
        sections.push(SectionOut { st, entries });
    }

    let safechars = ctx.cfg.biber.is_true("output_safechars");
    let outenc = output_encoding_name(ctx);
    let enc = if outenc == "UTF-8" {
        None
    } else {
        Some(find_output_encoding(ctx)?)
    };
    let recoder =
        (safechars || enc.is_some()).then(|| ctx.cfg.recoder(latex_recode::RecodeOptions::BIBER));
    // UnicodeOutputNulCheck: biber's NUL test misfires for UTF-16/32.
    let nul_check = ctx.cfg.quirks.on(Quirk::UnicodeOutputNulCheck);
    let unencodable = |e: Encoding, nul: bool, s: &str| {
        if nul {
            e.has_unencodable(s)
        } else {
            e.has_unencodable_char(s)
        }
    };
    let sortdebug = ctx.cfg.biber.is_true("sortdebug");
    let comma_quirk = ctx.cfg.quirks.on(Quirk::BblCommaCleanup);

    let mut out = String::new();
    let mut put = |s: &str| out.push_str(&nfc(s));

    // create_output_misc: \preamble (global, so the last section's).
    let mut headtext = head();
    if let Some(pa) = processed.sections.last().and_then(|s| s.preamble.as_ref()) {
        let mut pa = pa.join("%\n");
        if let Some(r) = &recoder {
            if safechars || enc.is_some_and(|e| unencodable(e, nul_check, &nfc(&pa))) {
                pa = r.encode(&pa);
            }
        }
        headtext.push_str(&format!("\\preamble{{%\n{pa}%\n}}\n\n"));
    }
    put(&headtext);

    // Sections with output entries, in string order of their numbers.
    let mut order: Vec<&SectionOut> = sections.iter().filter(|s| !s.entries.is_empty()).collect();
    order.sort_by(|a, b| a.st.section.number.cmp(&b.st.section.number));
    let mut collators: BTreeMap<Option<String>, ucollate::Collator> = BTreeMap::new();
    for so in order {
        let st = so.st;
        let secnum = &st.section.number;
        put(&format!("\n\\refsection{{{secnum}}}\n"));
        for dl in output_lists(ctx, st) {
            let keys = dl.keys.as_deref().unwrap_or(&[]);
            if keys.is_empty() {
                continue;
            }
            put(&format!("  \\datalist[{}]{{{}}}\n", dl.type_, dl.name));
            let loc = template_locale(ctx, &dl.sortingtemplatename);
            if !collators.contains_key(&loc) {
                let c = ucollate::Collator::new(
                    loc.as_deref(),
                    &ucollate::CollateOptions::biber_sortinit(),
                )
                .or_else(|_| {
                    ucollate::Collator::new(None, &ucollate::CollateOptions::biber_sortinit())
                })
                .expect("root collator");
                collators.insert(loc.clone(), c);
            }
            let coll = &collators[&loc];
            let hash = |si: &str| format!("{:x}", Md5::digest(coll.view_sort_key(si).as_bytes()));
            let lv = ListView {
                dl,
                state: dl.state.as_ref(),
                sortinithash: &hash,
            };
            for k in keys {
                let mut es = match (so.entries.get(k), st.section.bibentry(k)) {
                    (Some(t), Some(_)) => instantiate_entry(&lv, t, k),
                    _ => String::new(),
                };
                if comma_quirk {
                    es = dangling_comma_cleanup(&es);
                }
                if let Some(r) = &recoder {
                    if safechars {
                        es = r.encode(&es);
                    } else if let Some(e) = enc
                        && unencodable(e, nul_check, &nfc(&es))
                    {
                        es = r.encode(&es);
                        ctx.log.biber_warn(format!(
                            "The entry '{k}' has characters which cannot be encoded in '{outenc}'. Recoding problematic characters into macros."
                        ));
                    }
                }
                if sortdebug {
                    let sk = pd_path(dl.slots.get("sortdata"), &[k])
                        .and_then(|v| v.get(0))
                        .map(|v| pd_string(Some(v)))
                        .unwrap_or_default();
                    es = format!("    % sorting key for '{k}':\n    % {sk}\n{es}");
                }
                put(&es);
            }
            put("  \\enddatalist\n");
        }
        let mut aliases: Vec<(&String, &String)> = st.section.citekey_alias.iter().collect();
        aliases.sort();
        for (a, k) in aliases {
            put(&format!("  \\keyalias{{{a}}}{{{k}}}\n"));
        }
        let mut missing: Vec<&String> = st.section.undef_citekeys.iter().collect();
        missing.sort();
        missing.dedup();
        for k in missing {
            put(&format!("  \\missing{{{k}}}\n"));
        }
        put("\\endrefsection\n");
    }
    put("\\endinput\n\n");
    Ok(Some(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dangling_commas() {
        assert_eq!(dangling_comma_cleanup("a={b},}}%"), "a={b}}}%");
        assert_eq!(dangling_comma_cleanup("{Foo {bar,}}"), "{Foo {bar}}");
        assert_eq!(dangling_comma_cleanup("x,\n   }}y"), "x}}y");
        assert_eq!(dangling_comma_cleanup("x,\n}}"), "x,\n}}");
        assert_eq!(dangling_comma_cleanup("x,}y"), "x,}y");
    }

    #[test]
    fn join_names() {
        assert_eq!(join_name("van der"), "van\\bibnamedelimb der");
        assert_eq!(join_name("J.~R."), "J.\\bibnamedelimi R.");
        assert_eq!(join_name("a\\~b"), "a\\~b");
        assert_eq!(join_name("\\c  d"), "\\c \\bibnamedelimb d");
    }

    #[test]
    fn range_lengths() {
        let r = |a: &str, b: Option<&str>| vec![vec![Some(a.to_string()), b.map(str::to_string)]];
        assert_eq!(rangelen(&r("105", Some("10")), true), "11");
        assert_eq!(rangelen(&r("105", Some("10")), false), "6");
        assert_eq!(rangelen(&r("135", Some("38")), true), "4");
        assert_eq!(rangelen(&r("100", Some("")), true), "-1");
        assert_eq!(rangelen(&r("5", None), true), "1");
        assert_eq!(rangelen(&r("iv", Some("x")), true), "7");
    }

    #[test]
    fn removable_join() {
        let s = |x: &str| Some(x.to_string());
        assert_eq!(join_removable(&[s("a"), None]), "a,");
        assert_eq!(join_removable(&[None, s("a")]), "a");
        assert_eq!(join_removable(&[None, None]), "");
    }
}
