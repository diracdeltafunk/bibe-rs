//! Sorting: the sort data schema, per-entry sort keys, the multi-key
//! sort, and sortinit.
//!
//! A port of `Biber::generate_sortdataschema` (Biber.pm:3304),
//! `Biber::generate_sortinfo` (Biber.pm:3347) with the sort dispatch of
//! `Biber::Internals` (Internals.pm:1103-1785), and `Biber::sort_list`
//! (Biber.pm:4249, in [`collate`]). See `docs/research/04-sorting-collation.md`.

mod collate;
pub mod strings;
mod translit;

pub use collate::sort_list;

use super::Ctx;
use crate::bcf::templates::{
    Named, SnkPart, SortItemAttrs, SortPass, SortingNameKeyTemplate, SortingTemplate,
};
use crate::config::{Config, OptValue};
use crate::datamodel::DataModel;
use crate::log::Fatal;
use crate::model::{DataList, Entry, PerlData, Section, SectionState, Value};
use crate::perlish::{numify, truthy};
use crate::quirks::Quirk;
use perl_compat::grapheme::gcstring;
use perl_compat::regex::PerlRegex;
use serde_json::{Map, Value as J};
use strings::{normalise_string, normalise_string_sort, process_sort_attributes};

/// The sort separator joining the columns of a sort string
/// (`$sorting_sep`, Internals.pm:1103).
const SORTING_SEP: &str = ",";
/// Sort string for a truncated name/list ("et al."), sorting after everything.
const TRUNC: &str = "\u{10FFFD}";

/// The sorting templates (`getblxoption(undef, 'sortingtemplate')`).
fn sorting_templates(cfg: &Config) -> Option<&Named<SortingTemplate>> {
    match cfg.blx.global("sortingtemplate") {
        Some(OptValue::SortingTemplates(m)) => Some(m),
        _ => None,
    }
}

/// The sorting name key templates.
fn sortingnamekey_templates(cfg: &Config) -> Option<&Named<SortingNameKeyTemplate>> {
    match cfg.blx.global("sortingnamekeytemplate") {
        Some(OptValue::SortingNameKeyTemplates(m)) => Some(m),
        _ => None,
    }
}

/// The template a list sorts with (Biber.pm:3310-3317): the list's named
/// template if it has any content, else the global `sortingtemplatename`
/// one (BB-122: a missing name was autovivified empty).
pub(crate) fn list_sorting_template<'a>(
    cfg: &'a Config,
    dl: &DataList,
) -> Option<&'a SortingTemplate> {
    let m = sorting_templates(cfg)?;
    match m.get(&dl.sortingtemplatename) {
        Some(t) if !t.vivified => Some(t),
        _ => {
            let g = cfg.blx.global("sortingtemplatename")?.as_str()?;
            m.get(g)
        }
    }
}

/// The template as linked into the list (`set_sortingtemplate`): by the
/// time biber's dumps see it, `each $sort->[0]->%*` has autovivified every
/// pass's option hash and the `foreach` over `spec` the spec array.
fn linked_template_json(t: &SortingTemplate) -> PerlData {
    let mut j = t.to_json();
    if let J::Object(m) = &mut j {
        match m.get_mut("spec") {
            Some(J::Array(passes)) => {
                for p in passes {
                    if let J::Array(items) = p
                        && let Some(first) = items.first_mut()
                        && first.is_null()
                    {
                        *first = J::Object(Map::new());
                    }
                }
            }
            _ => {
                m.insert("spec".into(), J::Array(Vec::new()));
            }
        }
    }
    j
}

/// Whether a sort item makes its column an `int` column (`DataModel.pm:335`
/// `sortdataschema`). With `sort-column-type` (BB-042) fixed, the labeldate
/// pseudo-fields (which redirect to date parts) are integers too.
fn is_int_item(cfg: &Config, dm: &DataModel, item: &str) -> bool {
    if matches!(item, "intciteorder" | "citeorder" | "citecount")
        || dm.helpers.integers.iter().any(|f| f == item)
    {
        return true;
    }
    !cfg.quirks.on(Quirk::SortColumnType) && matches!(item, "labelyear" | "labelmonth" | "labelday")
}

/// `Biber::generate_sortdataschema` (Biber.pm:3304), for the data list
/// `st.datalists[list]`: link the sorting template into the list and work out
/// each column's type (`int`/`str`) and direction.
pub fn generate_sortdataschema(
    ctx: &mut Ctx,
    st: &mut SectionState,
    list: usize,
) -> Result<(), Fatal> {
    let dl = &mut st.datalists[list];
    let tmpl = list_sorting_template(ctx.cfg, dl);
    let mut ds = Vec::new();
    if let Some(t) = tmpl {
        for pass in &t.spec {
            let dir = if pass.opts.sort_direction.as_deref() == Some("descending") {
                "-"
            } else {
                ""
            };
            let first = pass.items.first().map_or("", |i| i.item.as_str());
            let ty = if is_int_item(ctx.cfg, ctx.dm, first) {
                "int"
            } else {
                "str"
            };
            let mut o = Map::new();
            o.insert("spec".into(), J::from(format!("{dir}{ty}")));
            o.insert(ty.into(), J::from("1"));
            ds.push(J::Object(o));
        }
    }
    let tj = match tmpl {
        Some(t) => linked_template_json(t),
        None => J::Object(Map::new()),
    };
    dl.slots.insert("sortingtemplate".into(), tj);
    dl.slots.insert(
        "sortdataschema".into(),
        if ds.is_empty() { J::Null } else { J::Array(ds) },
    );
    Ok(())
}

/// `Biber::generate_sortinfo` (Biber.pm:3347), for the data list
/// `st.datalists[list]`: compute every key's sort object
/// (`_generatesortinfo`, Internals.pm:1213) and sortinit.
pub fn generate_sortinfo(ctx: &mut Ctx, st: &mut SectionState, list: usize) -> Result<(), Fatal> {
    let empty = SortingTemplate {
        locale: None,
        spec: Vec::new(),
        vivified: true,
    };
    let keys = st.datalists[list].keys.clone().unwrap_or_default();
    let mut results = Vec::with_capacity(keys.len());
    let warnings = {
        let dl = &st.datalists[list];
        let tmpl = list_sorting_template(ctx.cfg, dl).unwrap_or(&empty);
        let mut g = Gen {
            cfg: ctx.cfg,
            dm: ctx.dm,
            sec: &st.section,
            secnum: &st.section.number,
            dl,
            config_state: &st.config_state,
            snk: sortingnamekey_templates(ctx.cfg),
            warnings: Vec::new(),
            viv: Default::default(),
        };
        for k in &keys {
            results.push((k.clone(), g.sortinfo(k, tmpl)));
        }
        (g.warnings, g.viv.into_inner())
    };
    let (warnings, viv) = warnings;
    for w in warnings {
        ctx.log.biber_warn(w);
    }
    apply_viv(st, list, viv);
    let dl = &mut st.datalists[list];
    let had_sd = dl.slots.contains_key("sortdata");
    let had_si = dl.slots.contains_key("sortinitdata");
    let mut sd = match dl.slots.remove("sortdata") {
        Some(J::Object(m)) => m,
        _ => Map::new(),
    };
    let mut si = match dl.slots.remove("sortinitdata") {
        Some(J::Object(m)) => m,
        _ => Map::new(),
    };
    for (k, info) in results {
        let Some(info) = info else { continue };
        sd.insert(
            k.clone(),
            J::Array(vec![
                J::from(info.string),
                J::Array(info.obj.into_iter().map(J::from).collect()),
            ]),
        );
        if let Some(init) = info.init {
            let mut o = Map::new();
            o.insert("init".into(), J::from(init));
            si.insert(k, J::Object(o));
        }
    }
    // The hashes only exist once something was stored in them.
    if !sd.is_empty() || had_sd {
        dl.slots.insert("sortdata".into(), J::Object(sd));
    }
    if !si.is_empty() || had_si {
        dl.slots.insert("sortinitdata".into(), J::Object(si));
    }
    Ok(())
}

/// The sort information of one key.
struct SortInfo {
    /// The sort string (`join(',', @sortobj)`).
    string: String,
    /// One value per sort set.
    obj: Vec<String>,
    /// sortinit, if the sort string is non-empty.
    init: Option<String>,
}

/// State for computing sort objects for one list.
struct Gen<'a> {
    cfg: &'a Config,
    dm: &'a DataModel,
    sec: &'a Section,
    secnum: &'a str,
    dl: &'a DataList,
    config_state: &'a PerlData,
    snk: Option<&'a Named<SortingNameKeyTemplate>>,
    warnings: Vec<String>,
    /// Hash entries Perl autovivifies while reading (applied afterwards).
    viv: std::cell::RefCell<Vec<Viv>>,
}

/// A hash entry biber autovivifies by reading through it.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Viv {
    /// `$CONFIG->{state}{keyorder|internalkeyorder}{$secnum}` (`get_keyorder`).
    ConfigState(&'static str),
    /// `$dlist->{state}{fields}{$citekey}` (`get_entryfield`).
    ListField(String),
    /// `$dlist->{state}{namelistdata}{$nlid}` (`get_visible_sort`).
    NameListData(String),
}

/// Create `m[k]` as `{}` if absent (`m` itself becomes a hash if needed).
fn vivify<'j>(m: &'j mut J, k: &str) -> &'j mut J {
    if !m.is_object() {
        *m = J::Object(Map::new());
    }
    let J::Object(o) = m else { unreachable!() };
    o.entry(k.to_string())
        .or_insert_with(|| J::Object(Map::new()))
}

/// Apply recorded autovivifications.
fn apply_viv(st: &mut SectionState, list: usize, viv: Vec<Viv>) {
    let secnum = st.section.number.clone();
    for v in viv {
        match v {
            Viv::ConfigState(which) => {
                vivify(vivify(&mut st.config_state, which), &secnum);
            }
            Viv::ListField(k) => {
                let s = st.datalists[list].state.get_or_insert_with(|| J::Object(Map::new()));
                vivify(vivify(s, "fields"), &k);
            }
            Viv::NameListData(k) => {
                let s = st.datalists[list].state.get_or_insert_with(|| J::Object(Map::new()));
                vivify(vivify(s, "namelistdata"), &k);
            }
        }
    }
}

/// Perl truthiness of an optional scalar result.
fn t(s: &Option<String>) -> bool {
    truthy(s.as_deref())
}

/// Perl truthiness of a field value (references are true).
fn value_true(v: Option<&Value>) -> bool {
    match v {
        None => false,
        Some(Value::Str(s)) => truthy(Some(s)),
        Some(_) => true,
    }
}

/// A defined scalar from a slot (`undef` → `None`).
fn slot_str<'a>(slots: &'a std::collections::BTreeMap<String, PerlData>, k: &str) -> Option<&'a str> {
    slots.get(k).and_then(J::as_str)
}

/// Perl numeric value of an optional scalar (`undef` → 0).
fn num(s: Option<&str>) -> f64 {
    s.map_or(0.0, numify)
}

/// Perl's stringification of an integral number.
fn fmt_num(x: f64) -> String {
    perl_compat::truth::format_nv(x)
}

impl<'a> Gen<'a> {
    fn quirk(&self, q: Quirk) -> bool {
        self.cfg.quirks.on(q)
    }

    /// `getblxoption($secnum, $opt, $entrytype, $citekey)` (defined values).
    fn opt(&self, opt: &str, et: Option<&str>, key: Option<&str>) -> Option<&'a OptValue> {
        self.cfg
            .blx
            .get(Some(self.secnum), opt, et, key)
            .filter(|v| v.is_defined())
    }

    fn opt_str(&self, opt: &str, et: Option<&str>, key: Option<&str>) -> Option<&'a str> {
        self.opt(opt, et, key).and_then(OptValue::as_str)
    }

    fn opt_true(&self, opt: &str, et: Option<&str>, key: Option<&str>) -> bool {
        self.opt(opt, et, key).is_some_and(OptValue::truthy)
    }

    /// Whether a scalar field value counts as present: Perl truthiness, so
    /// `0` is missing (BB-055, `zero-is-empty`), unless that quirk is fixed.
    fn present(&self, v: Option<&Value>) -> bool {
        match v {
            Some(Value::Str(s)) if !self.quirk(Quirk::ZeroIsEmpty) => !s.is_empty(),
            other => value_true(other),
        }
    }

    /// `_generatesortinfo` (Internals.pm:1213).
    fn sortinfo(&mut self, key: &str, tmpl: &SortingTemplate) -> Option<SortInfo> {
        let be = self.sec.bibentry(key)?;
        let mut final_ = String::new();
        let mut suppress = true;
        let mut szero = false;
        let mut obj = Vec::with_capacity(tmpl.spec.len());
        for pass in &tmpl.spec {
            let mut s = self.sortset(pass, key, be, &mut final_);
            if s == "BIBERZERO" {
                szero = true;
                s = "0".into();
            }
            if truthy(Some(&final_)) {
                obj.push(if suppress { String::new() } else { final_.clone() });
                suppress = false;
            } else {
                obj.push(s);
            }
        }
        let ss = obj.join(SORTING_SEP);
        let init = if truthy(Some(&ss)) || szero {
            let et = be.entrytype();
            let pre = self.opt_str("presort", Some(et), Some(key)).unwrap_or("");
            // `$ss =~ s/\A$pre$sorting_sep+//`: the presort value is
            // interpolated as a regex (BB-004: an invalid one would die;
            // we strip it literally instead, DC-004).
            let stripped = match PerlRegex::cached(&format!("\\A{pre}{SORTING_SEP}+"), false) {
                Ok(re) => match re.matches(&ss) {
                    Ok(ms) => match ms.first().and_then(|g| g[0]) {
                        Some((a, b)) => format!("{}{}", &ss[..a], &ss[b..]),
                        None => ss.clone(),
                    },
                    Err(_) => ss.clone(),
                },
                Err(_) => match ss.strip_prefix(pre) {
                    Some(r) if r.starts_with(SORTING_SEP) => {
                        r.trim_start_matches(SORTING_SEP).to_string()
                    }
                    _ => ss.clone(),
                },
            };
            Some(gcstring::first(&normalise_string(&stripped)).to_string())
        } else {
            None
        };
        Some(SortInfo {
            string: ss,
            obj,
            init,
        })
    }

    /// `_sortset` (Internals.pm:1275): the first true value among the
    /// pass's items; a `final` pass stashes its value and returns `''`.
    fn sortset(&mut self, pass: &SortPass, key: &str, be: &Entry, final_: &mut String) -> String {
        for item in &pass.items {
            let out = self.dispatch(&item.item, key, be, &item.attrs);
            if t(&out) {
                let out = out.unwrap_or_default();
                if truthy(pass.opts.final_.as_deref()) {
                    *final_ = out;
                    break;
                }
                return out;
            }
        }
        String::new()
    }

    /// `_dispatch_sorting` (Internals.pm:1167) with the dispatch table of
    /// `_dispatch_table_sorting` (Internals.pm:1122).
    fn dispatch(&mut self, field: &str, key: &str, be: &Entry, a: &SortItemAttrs) -> Option<String> {
        let et = be.entrytype();
        if let Some(OptValue::FieldSet(se)) = self.cfg.blx.get(None, "sortexclusion", Some(et), None)
            && se.contains(field)
        {
            return Some(String::new());
        }
        if let Some(OptValue::FieldSet(se)) = self.cfg.blx.get(None, "sortexclusion", Some("*"), None)
            && se.contains(field)
        {
            match self.cfg.blx.get(None, "sortinclusion", Some(et), None) {
                Some(OptValue::FieldSet(si)) if si.contains(field) => {}
                _ => return Some(String::new()),
            }
        }
        if truthy(a.literal.as_deref()) {
            return Some(process_sort_attributes(field, a));
        }
        match field {
            "editoratype" | "editorbtype" | "editorctype" => {
                return Some(self.sort_editort(field, key, be, a));
            }
            "citeorder" => return Some(self.sort_citeorder(key)),
            "citecount" => {
                let c = self
                    .sec
                    .slots
                    .get("citecount")
                    .and_then(|m| m.get(key))
                    .and_then(J::as_str)
                    .filter(|c| truthy(Some(c)))
                    .unwrap_or("-1");
                return Some(c.to_string());
            }
            "intciteorder" => {
                self.viv.borrow_mut().push(Viv::ConfigState("internalkeyorder"));
                return self
                    .config_state
                    .get("internalkeyorder")
                    .and_then(|m| m.get(self.secnum))
                    .and_then(|m| m.get(key))
                    .and_then(J::as_str)
                    .map(str::to_string);
            }
            "labelalpha" => {
                self.viv.borrow_mut().push(Viv::ListField(key.to_string()));
                let s = self
                    .dl
                    .state
                    .as_ref()
                    .and_then(|st| st.get("fields"))
                    .and_then(|f| f.get(key))
                    .and_then(|f| f.get("sortlabelalpha"))
                    .and_then(J::as_str)
                    .unwrap_or("");
                return Some(process_sort_attributes(s, a));
            }
            "labelname" | "labeltitle" => {
                let slot = if field == "labelname" {
                    "labelnameinfo"
                } else {
                    "labeltitleinfo"
                };
                return match slot_str(&be.slots, slot).filter(|s| truthy(Some(s))) {
                    Some(f) => {
                        let f = f.to_string();
                        self.dispatch(&f, key, be, a)
                    }
                    None => Some(String::new()),
                };
            }
            "labelyear" | "labelmonth" | "labelday" => {
                let ldc = &field[5..];
                let Some(ldi) = be.slots.get("labeldateinfo").filter(|v| !v.is_null()) else {
                    return Some(String::new());
                };
                if let Some(f) = ldi
                    .get("field")
                    .and_then(|f| f.get(ldc))
                    .and_then(J::as_str)
                    .filter(|s| truthy(Some(s)))
                {
                    let f = f.to_string();
                    return self.dispatch(&f, key, be, a);
                }
                return if ldi.get("string").is_some() {
                    Some(String::new())
                } else {
                    None
                };
            }
            "presort" => {
                let s = self.opt_str("presort", Some(et), Some(key)).unwrap_or("");
                return Some(process_sort_attributes(s, a));
            }
            "sortname" => return Some(self.sort_sortname(key, be, a)),
            "entrytype" => return Some(process_sort_attributes(et, a)),
            "entrykey" => return Some(process_sort_attributes(key, a)),
            _ => {}
        }
        let am = self.cfg.biber.annotation_marker();
        let nm = self.cfg.biber.named_annotation_marker();
        let fd = if self.dm.is_field(field, am, nm) {
            self.dm.field_id(field).map(|id| self.dm.field(id))
        } else {
            None
        };
        let kind = fd.map(|d| {
            (
                d.fieldtype.as_deref().unwrap_or(""),
                d.datatype.as_deref().unwrap_or(""),
            )
        });
        match kind {
            Some(("list", "name")) => Some(self.sort_name(field, key, be, a)),
            // Checked before the list cases (Internals.pm:1132), so
            // `_sort_list_verbatim` is unreachable: verbatim/uri *lists* reach
            // `_sort_verbatim` too.
            Some((_, "verbatim" | "uri")) => Some(self.sort_verbatim(field, be, a)),
            Some(("field", "literal" | "key")) => Some(self.sort_literal(field, be, a)),
            Some(("field", "integer" | "datepart")) => self.sort_integer(field, key, be, a),
            Some(("list", "literal" | "key")) => Some(self.sort_list(field, key, be, a)),
            _ => {
                self.warnings.push(format!(
                    "Field '{field}' in sorting template is not a sortable field"
                ));
                None
            }
        }
    }

    /// `_sort_citeorder` (Internals.pm:1304).
    fn sort_citeorder(&self, key: &str) -> String {
        self.viv.borrow_mut().push(Viv::ConfigState("keyorder"));
        let ko_map = self
            .config_state
            .get("keyorder")
            .and_then(|m| m.get(self.secnum));
        let ko = ko_map.and_then(|m| m.get(key)).and_then(J::as_str);
        if self.sec.allkeys {
            let max = ko_map
                .and_then(J::as_object)
                .map(|m| {
                    m.values()
                        .filter_map(J::as_str)
                        .map(numify)
                        .fold(None, |a: Option<f64>, v| Some(a.map_or(v, |a| a.max(v))))
                })
                .unwrap_or(None)
                .unwrap_or(0.0);
            let idx = self
                .sec
                .orig_order_citekeys
                .iter()
                .position(|k| k == key)
                .map_or(-1.0, |i| i as f64);
            let biborder = fmt_num(max + idx + 1.0);
            let ako = ko_map.and_then(|m| m.get("*")).and_then(J::as_str);
            if let (Some(ko), Some(ako)) = (ko, ako)
                && numify(ako) < numify(ko)
            {
                return biborder;
            }
            match ko {
                Some(k) if truthy(Some(k)) => k.to_string(),
                _ => biborder,
            }
        } else {
            match ko {
                Some(k) if truthy(Some(k)) => k.to_string(),
                _ => String::new(),
            }
        }
    }

    /// `_sort_editort` (Internals.pm:1361).
    fn sort_editort(&mut self, field: &str, key: &str, be: &Entry, a: &SortItemAttrs) -> String {
        let et = be.entrytype();
        if self.opt_true("useeditor", Some(et), Some(key)) && value_true(be.get_field(field)) {
            let s = be.get_str(field).unwrap_or("");
            let s = process_sort_attributes(s, a);
            return translit::translit(self.cfg, field, be, s, &mut self.warnings);
        }
        String::new()
    }

    /// `_sort_integer` (Internals.pm:1338).
    fn sort_integer(&self, field: &str, key: &str, be: &Entry, a: &SortItemAttrs) -> Option<String> {
        let v = be.get_field(field);
        if !self.present(v) {
            return Some(String::new());
        }
        let Some(Value::Str(f)) = v else {
            return Some(String::new());
        };
        let mut f: Option<String> = Some(f.clone());
        let nfkd = perl_compat::unicode::nfkd(f.as_deref().unwrap_or("")).into_owned();
        if perl_compat::roman::isroman(&nfkd)
            && !self.opt_true("noroman", Some(be.entrytype()), Some(key))
        {
            f = perl_compat::roman::roman2int(&nfkd).map(|n| n.to_string());
        }
        // `num($field) // $field`
        let f = f.map(|s| match perl_compat::unicode::ucd_num(&s) {
            Some(n) => n.to_string(),
            None => s,
        })?;
        Some(process_sort_attributes(&f, a))
    }

    /// `_sort_list` (Internals.pm:1440).
    fn sort_list(&mut self, field: &str, key: &str, be: &Entry, a: &SortItemAttrs) -> String {
        if !value_true(be.get_field(field)) {
            return String::new();
        }
        let s = self.liststring(key, be, field);
        let s = process_sort_attributes(&s, a);
        translit::translit(self.cfg, field, be, s, &mut self.warnings)
    }

    /// `_sort_literal` (Internals.pm:1472).
    fn sort_literal(&mut self, field: &str, be: &Entry, a: &SortItemAttrs) -> String {
        let v = be.get_field(field);
        if !self.present(v) {
            return String::new();
        }
        let s = normalise_string_sort(self.cfg, v.and_then(Value::as_str).unwrap_or(""), field);
        let s = process_sort_attributes(&s, a);
        translit::translit(self.cfg, field, be, s, &mut self.warnings)
    }

    /// `_sort_verbatim` (Internals.pm:1487).
    fn sort_verbatim(&mut self, field: &str, be: &Entry, a: &SortItemAttrs) -> String {
        let v = be.get_field(field);
        if !self.present(v) {
            return String::new();
        }
        let raw = match v {
            Some(Value::Str(s)) => s.clone(),
            // A verbatim/uri list stringifies as `ARRAY(0x…)` (a memory
            // address, so biber's order is arbitrary); use a fixed string.
            _ => "ARRAY".to_string(),
        };
        let s = normalise_string_sort(self.cfg, &raw, field);
        process_sort_attributes(&s, a)
    }

    /// `_sort_name` (Internals.pm:1501).
    fn sort_name(&mut self, field: &str, key: &str, be: &Entry, a: &SortItemAttrs) -> String {
        let et = be.entrytype();
        let useopt = format!("use{field}");
        if self.cfg.blx.registry.is_declared(&useopt) && !self.opt_true(&useopt, Some(et), Some(key)) {
            return String::new();
        }
        if !value_true(be.get_field(field)) {
            return String::new();
        }
        let s = self.namestring(key, be, field);
        let s = process_sort_attributes(&s, a);
        translit::translit(self.cfg, field, be, s, &mut self.warnings)
    }

    /// `_sort_sortname` (Internals.pm:1523).
    fn sort_sortname(&mut self, key: &str, be: &Entry, a: &SortItemAttrs) -> String {
        let et = be.entrytype();
        if value_true(be.get_field("sortname"))
            && self
                .dm
                .helpers
                .namelistsall
                .iter()
                .any(|n| self.opt_true(&format!("use{n}"), Some(et), Some(key)))
        {
            let s = self.namestring(key, be, "sortname");
            let s = process_sort_attributes(&s, a);
            return translit::translit(self.cfg, "sortname", be, s, &mut self.warnings);
        }
        String::new()
    }

    /// `$section->get_np_length($np)`.
    fn np_length(&self, np: &str) -> f64 {
        num(self
            .sec
            .slots
            .get("namepartlengths")
            .and_then(|m| m.get(np))
            .and_then(J::as_str))
    }

    /// `_namestring` (Internals.pm:1589). Per-name and per-namelist
    /// `useprefix` and `sortingnamekeytemplatename` overrides leak to the
    /// following names (BB-019, `per-name-option-leak`).
    fn namestring(&self, key: &str, be: &Entry, field: &str) -> String {
        let Some(Value::Names(nlid)) = be.get_field(field) else {
            return String::new();
        };
        let leak = self.quirk(Quirk::PerNameOptionLeak);
        let et = be.entrytype();
        let names = self.sec.names.list(*nlid);
        let count = names.names.len();
        let mut useprefix: Option<String> = self.opt_str("useprefix", Some(et), Some(key)).map(str::to_string);
        let mut snkname = self.dl.sortingnamekeytemplatename.clone();
        if let Some(v) = self
            .cfg
            .blx
            .get(Some(self.secnum), "sortingnamekeytemplatename", None, Some(key))
            .and_then(OptValue::as_str)
        {
            snkname = v.to_string();
        }
        if let Some(v) = slot_str(&names.slots, "sortingnamekeytemplatename") {
            snkname = v.to_string();
        }
        let tmpsnk = self.snk.and_then(|m| m.get(&snkname));
        self.viv.borrow_mut().push(Viv::NameListData(nlid.key()));
        let nld = self
            .dl
            .state
            .as_ref()
            .and_then(|s| s.get("namelistdata"))
            .and_then(|m| m.get(nlid.key()));
        let vis_key = if tmpsnk.is_some_and(|t| t.visibility.as_deref() == Some("cite")) {
            "viscite"
        } else {
            "vissort"
        };
        let visible = num(nld.and_then(|d| d.get(vis_key)).and_then(J::as_str));
        if let Some(u) = slot_str(&names.slots, "useprefix") {
            useprefix = Some(u.to_string());
        }
        let size = count as f64;
        let take = if visible > size {
            count
        } else if visible >= 1.0 {
            visible as usize
        } else {
            0
        };
        let mut out = String::new();
        for nid in names.names.iter().take(take) {
            let n = self.sec.names.name(*nid);
            let mut n_useprefix = useprefix.clone();
            if let Some(u) = slot_str(&n.slots, "useprefix") {
                n_useprefix = Some(u.to_string());
            }
            let mut n_snkname = snkname.clone();
            if let Some(s) = slot_str(&n.slots, "sortingnamekeytemplatename") {
                n_snkname = s.to_string();
            }
            if leak {
                useprefix = n_useprefix.clone();
                snkname = n_snkname.clone();
            }
            let Some(snk) = self.snk.and_then(|m| m.get(&n_snkname)) else {
                continue;
            };
            for kp in &snk.template {
                let mut kps = String::new();
                for np in kp {
                    match np {
                        SnkPart::Namepart { value, use_, inits } => {
                            let part = value.as_deref().unwrap_or("");
                            let useopt = use_.as_ref().map(|_| format!("use{part}"));
                            let useoptval: Option<String> = match useopt.as_deref() {
                                Some("useprefix") => self
                                    .cfg
                                    .blx
                                    .registry
                                    .map_boolean_tonum("useprefix", n_useprefix.as_deref()),
                                Some(o) => self.opt_str(o, Some(et), Some(key)).map(str::to_string),
                                None => None,
                            };
                            let Some(np_data) = n.nameparts.get(part) else {
                                continue;
                            };
                            let npstring = np_data.string.as_deref().unwrap_or("");
                            if !truthy(Some(npstring)) {
                                continue;
                            }
                            let use_val = use_.as_ref().and_then(|u| u.as_deref());
                            if useopt.is_some() && num(useoptval.as_deref()) != num(use_val) {
                                continue;
                            }
                            let nps = if truthy(inits.as_ref().and_then(|i| i.as_deref())) {
                                let inits: String =
                                    np_data.initial.as_deref().unwrap_or(&[]).concat();
                                let s = normalise_string_sort(self.cfg, &inits, field);
                                strings::pad_namepart(&s, self.np_length(&format!("{part}-i")))
                            } else {
                                let s = normalise_string_sort(self.cfg, npstring, field);
                                strings::pad_namepart(&s, self.np_length(part))
                            };
                            kps.push_str(&nps);
                        }
                        SnkPart::Literal { value } => kps.push_str(value.as_deref().unwrap_or("")),
                        SnkPart::Undef => {}
                    }
                }
                if truthy(Some(&kps)) {
                    out.push_str(&kps);
                }
            }
        }
        let mut nso = self.opt_true("nosortothers", Some(et), Some(key));
        if let Some(v) = names.slots.get("nosortothers").filter(|v| !v.is_null()) {
            nso = truthy(v.as_str());
        }
        if !nso && visible < size {
            out.push_str(TRUNC);
        }
        out
    }

    /// `_liststring` (Internals.pm:1722) for non-verbatim lists (the
    /// verbatim variant is unreachable from sorting, see `dispatch`).
    fn liststring(&self, key: &str, be: &Entry, field: &str) -> String {
        let et = be.entrytype();
        let Some(Value::List(items)) = be.get_field(field) else {
            return String::new();
        };
        let maxitems = num(self.opt_str("maxitems", Some(et), Some(key)));
        let mut items: &[String] = items;
        let mut truncated = false;
        if items.len() as f64 > maxitems {
            truncated = true;
            let min = num(self.opt_str("minitems", Some(et), Some(key)));
            let n = if min <= 0.0 { 0 } else { (min as usize).min(items.len()) };
            items = &items[..n];
        }
        let parts: Vec<String> = items
            .iter()
            .map(|i| normalise_string_sort(self.cfg, i, field))
            .collect();
        let mut s = parts.join("!");
        let trimmed = s.trim_end_matches(perl_compat::unicode::is_perl_space).len();
        s.truncate(trimmed);
        if truncated {
            s.push_str(TRUNC);
        }
        s
    }
}
