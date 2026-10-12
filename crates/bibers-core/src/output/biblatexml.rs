//! `Biber::Output::biblatexml` (vendor/biber @ v2.22,
//! `lib/Biber/Output/biblatexml.pm`, with `Biber::Entry::Name::
//! name_to_biblatexml`/`name_part_to_bltxml`): biblatexml output, used only
//! in tool mode (`--tool --output-format=biblatexml`).
//!
//! Not wired into the program yet: tool mode (DC-030) is another package.
//! Wiring: after tool-mode processing of the section (biber's
//! `create_output_section` iterates `get_citekeys`, i.e. *input* order,
//! BB-114), call [`BltxmlWriter::new`], [`BltxmlWriter::section`],
//! [`BltxmlWriter::finish`], write the result with the output encoding, then
//! [`schema_file_for_dsn`] + `bltxml_schema::generate_bltxml_schema` unless
//! `--no-bltxml-schema` (Output/biblatexml.pm:571-590).
//!
//! biber iterates three Perl hashes here, so its attribute and option order
//! varies between runs: per-namelist options and per-name options (as
//! attributes) use the `.bcf` declaration order of the option scopes
//! (DC-013's rule), and per-entry options (`<bltx:options>`) are sorted.

use crate::config::blx::{OptValue, Scope, map_boolean_tostring_raw};
use crate::model::{Entry, NameId, Section, SectionState, Value};
use crate::perlish::{lc, nfc};
use crate::pipeline::Ctx;
use crate::xmlwriter::{XName, XmlWriter};
use serde_json::Value as J;

const BLTX_NS: &str = "http://biblatex-biber.sourceforge.net/biblatexml";

/// The schema file for a tool-mode data source name
/// (`$dsn =~ s/\.(?:bib|bltxml)$/.rng/r`). DC-010 (BB-012): when the name
/// has neither extension biber's result is the input name itself, so the
/// schema would overwrite the input; bibers appends `.rng` instead.
pub fn schema_file_for_dsn(dsn: &str) -> String {
    for ext in [".bib", ".bltxml"] {
        if let Some(stem) = dsn.strip_suffix(ext) {
            return format!("{stem}.rng");
        }
    }
    format!("{dsn}.rng")
}

/// The writer (`set_output_target_file` … `output`).
pub struct BltxmlWriter {
    w: XmlWriter,
}

fn n(local: &str) -> XName<'_> {
    XName::Ns(BLTX_NS, local)
}

/// `date_monthday`: `sprintf('%.2d', $md)` for a true value, else undef.
fn date_monthday(md: Option<&str>) -> Option<String> {
    let md = md.filter(|m| perl_compat::perl_truthy(m))?;
    Some(sprintf_2d(md))
}

/// `sprintf('%.2d', $v)` (numified, truncated).
fn sprintf_2d(v: &str) -> String {
    let x = perl_compat::truth::numify(v).trunc() as i64;
    if x < 0 {
        format!("-{:02}", -x)
    } else {
        format!("{x:02}")
    }
}

/// `xdatarefout($ref, 1)`: strip the input marker and separator, and map the
/// XDATA separator to the output one.
fn xdatarefout(ctx: &Ctx, xref: &str) -> String {
    let b = &ctx.cfg.biber;
    let xdmi = b.get_str("xdatamarker").unwrap_or("");
    let xnsi = b.get_str("xnamesep").unwrap_or("");
    let xdsi = b.get_str("xdatasep").unwrap_or("");
    let xdso = b.get_str("output_xdatasep").unwrap_or("");
    let prefix = format!("{xdmi}{xnsi}");
    let s = xref.strip_prefix(prefix.as_str()).unwrap_or(xref);
    if xdsi.is_empty() {
        s.to_string()
    } else {
        s.replace(xdsi, xdso)
    }
}

/// `xdatarefcheck($val, 1)` on a scalar (array values never match: biber
/// tests their stringified reference).
fn xdatarefcheck(ctx: &Ctx, val: Option<&Value>) -> Option<String> {
    let Some(Value::Str(v)) = val else {
        return None;
    };
    if !perl_compat::perl_truthy(v) {
        return None;
    }
    let b = &ctx.cfg.biber;
    let lead = format!(
        "{}{}",
        b.get_str("xdatamarker").unwrap_or(""),
        b.get_str("xnamesep").unwrap_or("")
    );
    let t = v.trim_start_matches(perl_compat::unicode::is_perl_space);
    t.starts_with(&lead).then(|| xdatarefout(ctx, v))
}

/// `Biber::Entry::is_xdata_resolved($field, $pos)`.
fn is_xdata_resolved(be: &Entry, field: &str, pos: Option<usize>) -> bool {
    let Some(J::Array(refs)) = be.slots.get("xdatarefs") else {
        return false;
    };
    for r in refs {
        if r.get("reffield").and_then(J::as_str) != Some(field) {
            continue;
        }
        if let Some(p) = pos {
            let rp = r.get("refposition").and_then(J::as_str).unwrap_or("");
            if perl_compat::truth::numify(rp) != p as f64 {
                continue;
            }
        }
        return match r.get("resolved") {
            Some(J::String(s)) => perl_compat::perl_truthy(s),
            Some(J::Bool(b)) => *b,
            Some(J::Number(x)) => x.as_f64().is_some_and(|f| f != 0.0),
            _ => false,
        };
    }
    false
}

fn str_of(v: Option<&Value>) -> Option<&str> {
    v.and_then(Value::as_str)
}

fn truthy(v: Option<&str>) -> bool {
    v.is_some_and(perl_compat::perl_truthy)
}

impl BltxmlWriter {
    /// `set_output_target_file($toolfile, 1)`: the document head.
    /// `schemafile` is [`schema_file_for_dsn`] of the data source.
    pub fn new(ctx: &Ctx, schemafile: &str) -> BltxmlWriter {
        let indent = ctx.cfg.biber.get_str("output_indent").unwrap_or("2");
        let mut w = XmlWriter::new(indent, &[(BLTX_NS, "bltx")]);
        w.xml_decl();
        w.pi(
            "xml-model",
            &format!(
                "href=\"{schemafile}\" type=\"application/xml\" schematypens=\"http://relaxng.org/ns/structure/1.0\""
            ),
        );
        w.comment("Auto-generated by Biber::Output::biblatexml");
        w.start_tag(n("entries"), &[]);
        BltxmlWriter { w }
    }

    /// `create_output_section`: every citekey of the section, in order. A
    /// citekey without an entry is biber's "Cannot find entry … to output"
    /// error; it is returned for the caller to report.
    pub fn section(&mut self, ctx: &Ctx, st: &SectionState) -> Vec<String> {
        let mut missing = Vec::new();
        for k in &st.section.citekeys {
            if st.section.entries.contains_key(k) {
                self.entry(ctx, st, k);
            } else {
                missing.push(k.clone());
            }
        }
        missing
    }

    /// `output`: close the document.
    pub fn finish(mut self) -> String {
        self.w.end_tag();
        self.w.finish()
    }

    /// `set_output_entry` (Output/biblatexml.pm:99).
    pub fn entry(&mut self, ctx: &Ctx, st: &SectionState, key: &str) {
        let sec = &st.section;
        let Some(be) = sec.entries.get(key) else {
            return;
        };
        let dm = ctx.dm;
        let b = &ctx.cfg.biber;
        let secnum = sec.number.as_str();
        let bee = be.entrytype().to_string();
        let w = &mut self.w;
        let id = nfc(key);
        let et = nfc(&bee);
        w.start_tag(n("entry"), &[("id", &id), ("entrytype", &et)]);

        // aliases pointing to this key
        let mut ids: Vec<&String> = sec
            .citekey_alias
            .iter()
            .filter(|(_, v)| v.as_str() == key)
            .map(|(k, _)| k)
            .collect();
        ids.sort();
        if !ids.is_empty() {
            w.start_tag(n("ids"), &[]);
            w.start_tag(n("list"), &[]);
            for i in ids {
                w.data_element(n("item"), &nfc(i), &[]);
            }
            w.end_tag();
            w.end_tag();
        }

        let resolve_xdata = b.is_true("output_resolve_xdata");
        if !resolve_xdata && let Some(Value::List(xd)) = be.get_field("xdata") {
            w.start_tag(n("xdata"), &[]);
            w.start_tag(n("list"), &[]);
            for x in xd {
                w.data_element(n("item"), &nfc(x), &[]);
            }
            w.end_tag();
            w.end_tag();
        }
        if !b.is_true("output_resolve_crossrefs")
            && let Some(cr) =
                str_of(be.get_field("crossref")).filter(|c| perl_compat::perl_truthy(c))
        {
            w.data_element(n("crossref"), &nfc(cr), &[]);
        }

        // per-entry options (hash order in biber: sorted)
        let opts: Vec<String> = ctx
            .cfg
            .blx
            .entry_options(secnum, key)
            .into_iter()
            .map(|o| {
                let v = match ctx.cfg.blx.get(Some(secnum), o, None, Some(key)) {
                    Some(OptValue::Str(s)) => s.clone(),
                    _ => String::new(),
                };
                format!("{o}={v}")
            })
            .collect();
        if !opts.is_empty() {
            w.data_element(n("options"), &nfc(&opts.join(",")), &[]);
        }

        // name fields
        let reg = &ctx.cfg.blx.registry;
        let others = b.get_str("others_string").unwrap_or("");
        for nf in dm.get_fields_of_type("list", &["name"], None) {
            let Some(Value::Names(lid)) = be.get_field(&nf) else {
                continue;
            };
            let nl = sec.names.list(*lid);
            if (!resolve_xdata || !is_xdata_resolved(be, &nf, None))
                && let Some(x) = nl
                    .slots
                    .get("xdata")
                    .and_then(J::as_str)
                    .filter(|x| !x.is_empty())
            {
                let v = nfc(&xdatarefout(ctx, x));
                w.empty_tag(n("names"), &[("xdata", &v)]);
                continue;
            }
            let mut attrs: Vec<(String, String)> = vec![("type".into(), nf.clone())];
            if nl
                .slots
                .get("morenames")
                .and_then(J::as_str)
                .is_some_and(perl_compat::perl_truthy)
            {
                attrs.push(("morenames".into(), "1".into()));
            }
            for nlo in reg.options_in_scope(Scope::NameList) {
                let Some(v) = nl.slots.get(nlo).and_then(J::as_str) else {
                    continue;
                };
                if reg
                    .decl(nlo, Scope::NameList)
                    .is_some_and(|d| perl_compat::perl_truthy(&d.output))
                {
                    attrs.push((nlo.clone(), map_boolean(ctx, nlo, v)));
                }
            }
            let a: Vec<(&str, &str)> = attrs
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            w.start_tag(n("names"), &a);
            for (i, nid) in nl.names.iter().enumerate() {
                let name = sec.names.name(*nid);
                if (!resolve_xdata || !is_xdata_resolved(be, &nf, Some(i + 1)))
                    && let Some(x) = name
                        .slots
                        .get("xdata")
                        .and_then(J::as_str)
                        .filter(|x| !x.is_empty())
                {
                    let v = nfc(&xdatarefout(ctx, x));
                    w.empty_tag(n("name"), &[("xdata", &v)]);
                    continue;
                }
                name_to_biblatexml(ctx, sec, w, *nid, key, &nf);
            }
            w.end_tag();
        }

        // list fields
        for lf in dm.get_fields_of_fieldtype("list") {
            if dm.field_is_datatype("name", &lf) {
                continue;
            }
            let Some(Value::List(items)) = be.get_field(&lf) else {
                continue;
            };
            let mut items: Vec<String> = items.clone();
            let mut attrs: Vec<(&str, &str)> = Vec::new();
            if items.last().is_some_and(|l| lc(l) == others) {
                attrs.push(("morelist", "1"));
                items.pop();
            }
            w.start_tag(n(&lf), &attrs);
            w.start_tag(n("list"), &[]);
            for (i, it) in items.iter().enumerate() {
                if !resolve_xdata || !is_xdata_resolved(be, &lf, Some(i + 1)) {
                    let v = Value::Str(it.clone());
                    if let Some(x) = xdatarefcheck(ctx, Some(&v)) {
                        w.empty_tag(n("item"), &[("xdata", &nfc(&x))]);
                        continue;
                    }
                }
                w.data_element(n("item"), &nfc(it), &[]);
            }
            w.end_tag();
            w.end_tag();
        }

        // standard fields
        for f in dm.get_fields_of_type(
            "field",
            &[
                "entrykey", "key", "literal", "code", "integer", "verbatim", "uri",
            ],
            None,
        ) {
            let val = be.get_field(&f);
            if (!resolve_xdata || !is_xdata_resolved(be, &f, None))
                && let Some(x) = xdatarefcheck(ctx, val)
            {
                w.empty_tag(n(&f), &[("xdata", &nfc(&x))]);
                continue;
            }
            let has_len = match val {
                Some(Value::Str(s)) => !s.is_empty(),
                Some(_) => true,
                None => false,
            };
            if has_len || (dm.field_is_nullok(&f) && be.field_exists(&f)) {
                if dm.get_fieldformat(&f) == Some("xsv") || f == "crossref" {
                    continue;
                }
                let s = str_of(val).unwrap_or("");
                w.data_element(n(&f), &nfc(s), &[]);
            }
        }

        // xsv fields: `get_fields_of_type('field', 'xsv')` asks for a
        // *datatype* `xsv`, which does not exist, so none are written

        // range fields
        for rf in dm.get_fields_of_datatype(&["range"]) {
            // An unresolved XDATA reference read from biblatexml is a
            // one-item list (`[$xdatav]`): biber's `xdatarefcheck` tests the
            // array reference and misses it, then dies dereferencing the
            // string as a range; write the reference, as intended.
            if let Some(Value::List(l)) = be.get_field(&rf) {
                if (!resolve_xdata || !is_xdata_resolved(be, &rf, None))
                    && let Some(x) =
                        xdatarefcheck(ctx, l.first().map(|s| Value::Str(s.clone())).as_ref())
                {
                    w.empty_tag(n(&rf), &[("xdata", &nfc(&x))]);
                }
                continue;
            }
            let Some(Value::Ranges(rs)) = be.get_field(&rf) else {
                continue;
            };
            w.start_tag(n(&rf), &[]);
            w.start_tag(n("list"), &[]);
            for r in rs {
                w.start_tag(n("item"), &[]);
                let start = r.first().cloned().flatten().unwrap_or_default();
                match r.get(1).cloned().flatten() {
                    Some(end) => {
                        w.data_element(n("start"), &nfc(&start), &[]);
                        w.data_element(n("end"), &nfc(&end), &[]);
                    }
                    None => w.characters(&nfc(&start)),
                }
                w.end_tag();
            }
            w.end_tag();
            w.end_tag();
        }

        // date fields
        for df in dm.get_fields_of_datatype(&["date"]) {
            let d = df.strip_suffix("date").unwrap_or(&df).to_string();
            write_date(w, be, &d);
        }

        // annotations
        let ann = &ctx.run.annotations;
        let lit = |l: bool| if l { "1" } else { "0" };
        if let Some(fields) = ann.field.get(key) {
            for (f, names) in fields {
                for (nm, a) in names {
                    w.data_element(
                        n("annotation"),
                        a.value.as_deref().unwrap_or(""),
                        &[("field", f), ("name", nm), ("literal", lit(a.literal))],
                    );
                }
            }
        }
        if let Some(fields) = ann.item.get(key) {
            for (f, names) in fields {
                for (nm, counts) in names {
                    let mut cs: Vec<&String> = counts.keys().collect();
                    cs.sort();
                    for c in cs {
                        let a = &counts[c];
                        w.data_element(
                            n("annotation"),
                            a.value.as_deref().unwrap_or(""),
                            &[
                                ("field", f),
                                ("name", nm),
                                ("item", c),
                                ("literal", lit(a.literal)),
                            ],
                        );
                    }
                }
            }
        }
        if let Some(fields) = ann.part.get(key) {
            for (f, names) in fields {
                for (nm, counts) in names {
                    for (c, parts) in counts {
                        for (p, a) in parts {
                            w.data_element(
                                n("annotation"),
                                a.value.as_deref().unwrap_or(""),
                                &[
                                    ("field", f),
                                    ("name", nm),
                                    ("item", c),
                                    ("part", p),
                                    ("literal", lit(a.literal)),
                                ],
                            );
                        }
                    }
                }
            }
        }
        w.end_tag(); // entry
    }
}

/// `map_boolean($opt, $v, 'tostring')`.
fn map_boolean(ctx: &Ctx, opt: &str, v: &str) -> String {
    if ctx.cfg.blx.registry.datatype(opt) == Some("boolean") {
        map_boolean_tostring_raw(Some(v)).unwrap_or_default()
    } else {
        v.to_string()
    }
}

/// `name_to_biblatexml` (Name.pm:221) and `name_part_to_bltxml`
/// (Name.pm:264).
fn name_to_biblatexml(
    ctx: &Ctx,
    sec: &Section,
    w: &mut XmlWriter,
    nid: NameId,
    key: &str,
    nf: &str,
) {
    let name = sec.names.name(nid);
    let reg = &ctx.cfg.blx.registry;
    let count = name.index.clone().unwrap_or_default();
    let mut attrs: Vec<(String, String)> = Vec::new();
    for no in reg.options_in_scope(Scope::Name) {
        let Some(v) = name.slots.get(no).and_then(J::as_str) else {
            continue;
        };
        if reg
            .decl(no, Scope::Name)
            .is_some_and(|d| perl_compat::perl_truthy(&d.output))
        {
            attrs.push((no.clone(), map_boolean(ctx, no, v)));
        }
    }
    if let Some(g) = name
        .slots
        .get("gender")
        .and_then(J::as_str)
        .filter(|g| perl_compat::perl_truthy(g))
    {
        attrs.push(("gender".into(), g.to_string()));
    }
    // `get_annotation('item', $key, $namefield, $count)`: the count is
    // passed as the annotation *name* (so this normally finds nothing)
    let ann = &ctx.run.annotations;
    let nm = if perl_compat::perl_truthy(&count) {
        count.as_str()
    } else {
        "default"
    };
    if let Some(v) = ann
        .item
        .get(key)
        .and_then(|m| m.get(nf))
        .and_then(|m| m.get(nm))
        .and_then(|m| m.get(""))
        .and_then(|a| a.value.clone())
        .filter(|v| perl_compat::perl_truthy(v))
    {
        attrs.push(("annotation".into(), v));
    }
    let a: Vec<(&str, &str)> = attrs
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    w.start_tag(n("name"), &a);
    for np in ctx.dm.get_constant_value("nameparts") {
        let Some(part) = name.nameparts.get(&np) else {
            continue;
        };
        let Some(s) = part
            .string
            .as_deref()
            .filter(|s| perl_compat::perl_truthy(s))
        else {
            continue;
        };
        let inits = part.initial.clone().unwrap_or_default();
        // split(/[\s~]/, $np): trailing empty fields dropped
        let mut parts: Vec<&str> = s
            .split(|c: char| c == '~' || perl_compat::unicode::is_perl_space(c))
            .collect();
        while parts.last().is_some_and(|p| p.is_empty()) {
            parts.pop();
        }
        let mut pattrs: Vec<(String, String)> = Vec::new();
        // `get_annotation('part', $key, $namefield, $count, $npn)`
        if let Some(v) = ann
            .part
            .get(key)
            .and_then(|m| m.get(nf))
            .and_then(|m| m.get(nm))
            .and_then(|m| m.get(&np))
            .and_then(|m| m.get(""))
            .and_then(|a| a.value.clone())
            .filter(|v| perl_compat::perl_truthy(v))
        {
            pattrs.push(("annotation".into(), v));
        }
        let init = |i: usize| {
            inits
                .get(i)
                .filter(|x| perl_compat::perl_truthy(x))
                .cloned()
        };
        if parts.len() > 1 {
            let mut a: Vec<(&str, &str)> = vec![("type", np.as_str())];
            a.extend(pattrs.iter().map(|(k, v)| (k.as_str(), v.as_str())));
            w.start_tag(n("namepart"), &a);
            for (i, p) in parts.iter().enumerate() {
                match init(i) {
                    Some(ini) => w.start_tag(n("namepart"), &[("initial", &ini)]),
                    None => w.start_tag(n("namepart"), &[]),
                }
                w.characters(&nfc(p));
                w.end_tag();
            }
            w.end_tag();
        } else {
            let first = parts.first().copied().unwrap_or("");
            let ini = init(0);
            let mut a: Vec<(&str, &str)> = vec![("type", np.as_str())];
            if let Some(ini) = &ini {
                a.push(("initial", ini));
            }
            a.extend(pattrs.iter().map(|(k, v)| (k.as_str(), v.as_str())));
            w.start_tag(n("namepart"), &a);
            w.characters(&nfc(first));
            w.end_tag();
        }
    }
    w.end_tag();
}

/// One date field (Output/biblatexml.pm:375-526).
fn write_date(w: &mut XmlWriter, be: &Entry, d: &str) {
    let g = |f: &str| be.get_field(&format!("{d}{f}")).and_then(Value::as_str);
    let Some(mut sf) = g("year")
        .filter(|y| perl_compat::perl_truthy(y))
        .map(str::to_string)
    else {
        return;
    };
    let mut attrs: Vec<(&str, &str)> = Vec::new();
    if !d.is_empty() {
        attrs.push(("type", d));
    }
    w.start_tag(n("date"), &attrs);
    if truthy(g("dateuncertain")) && truthy(g("dateapproximate")) {
        sf.push('%');
    } else {
        if truthy(g("dateuncertain")) {
            sf.push('?');
        }
        if truthy(g("dateapproximate")) {
            sf.push('~');
        }
    }
    if truthy(g("dateunknown")) {
        sf = "unknown".into();
    }
    let yd = |s: &str| -> Option<String> {
        Some(
            match s {
                "spring" => "21",
                "summer" => "22",
                "autumn" => "23",
                "winter" => "24",
                "springN" => "25",
                "summerN" => "26",
                "autumnN" => "27",
                "winterN" => "28",
                "springS" => "29",
                "summerS" => "30",
                "autumnS" => "31",
                "WinterS" => "32",
                "Q1" => "33",
                "Q2" => "34",
                "Q3" => "35",
                "Q4" => "36",
                "QD1" => "37",
                "QD2" => "38",
                "QD3" => "39",
                "S1" => "40",
                "S2" => "41",
                _ => return None,
            }
            .to_string(),
        )
    };
    let mut overridey: Option<String> = None;
    let mut overridem: Option<String> = None;
    let mut overrideem: Option<String> = None;
    let mut overrided: Option<String> = None;
    // `del_field` of end parts for unspecified dates (on a copy here)
    let mut dropped: Vec<&str> = Vec::new();
    if let Some(u) = g("dateunspecified").filter(|u| perl_compat::perl_truthy(u)) {
        let year = g("year").unwrap_or("");
        match u {
            "yearindecade" => {
                // m/^(\d+)\d$/
                let ds: Vec<char> = year.chars().collect();
                let ok =
                    ds.len() >= 2 && ds.iter().all(|c| perl_compat::unicode::is_perl_digit(*c));
                overridey = Some(format!(
                    "{}X",
                    if ok {
                        ds[..ds.len() - 1].iter().collect()
                    } else {
                        String::new()
                    }
                ));
                dropped.push("endyear");
            }
            "yearincentury" => {
                let ds: Vec<char> = year.chars().collect();
                let ok =
                    ds.len() >= 3 && ds.iter().all(|c| perl_compat::unicode::is_perl_digit(*c));
                overridey = Some(format!(
                    "{}XX",
                    if ok {
                        ds[..ds.len() - 2].iter().collect()
                    } else {
                        String::new()
                    }
                ));
                dropped.push("endyear");
            }
            "monthinyear" => {
                overridem = Some("XX".into());
                dropped.extend(["endyear", "endmonth"]);
            }
            "dayinmonth" => {
                overrided = Some("XX".into());
                dropped.extend(["endyear", "endmonth", "endday"]);
            }
            "dayinyear" => {
                overridem = Some("XX".into());
                overrided = Some("XX".into());
                dropped.extend(["endyear", "endmonth", "endday"]);
            }
            _ => {}
        }
    }
    if let Some(s) = g("yeardivision").filter(|s| perl_compat::perl_truthy(s)) {
        overridem = yd(s);
    }
    if let Some(s) = g("endyeardivision").filter(|s| perl_compat::perl_truthy(s)) {
        overrideem = yd(s);
    }
    let gd = |f: &str| if dropped.contains(&f) { None } else { g(f) };
    if let Some(y) = overridey.filter(|y| perl_compat::perl_truthy(y)) {
        sf = y;
    }
    let pick = |o: &Option<String>, f: &str| -> Option<String> {
        match o.as_deref().filter(|x| perl_compat::perl_truthy(x)) {
            Some(x) => date_monthday(Some(x)),
            None => date_monthday(gd(f)),
        }
    };
    let start: Vec<String> = [
        Some(sf.clone()),
        pick(&overridem, "month"),
        pick(&overrided, "day"),
    ]
    .into_iter()
    .flatten()
    .filter(|s| perl_compat::perl_truthy(s))
    .collect();
    let end: Vec<String> = [
        gd("endyear").map(str::to_string),
        pick(&overrideem, "endmonth"),
        date_monthday(gd("endday")),
    ]
    .into_iter()
    .flatten()
    .collect();
    let time = |h: &str, m: &str, s: &str| -> Option<String> {
        let hv = gd(h).filter(|x| perl_compat::perl_truthy(x))?;
        Some(format!(
            "T{}:{}:{}",
            sprintf_2d(hv),
            sprintf_2d(gd(m).unwrap_or("")),
            sprintf_2d(gd(s).unwrap_or(""))
        ))
    };
    let tz = |f: &str| -> Option<String> {
        let t = gd(f).filter(|x| perl_compat::perl_truthy(x))?;
        // s/\\bibtzminsep\s+/:/
        Some(match t.find("\\bibtzminsep") {
            Some(p) => {
                let rest = &t[p + "\\bibtzminsep".len()..];
                let trimmed = rest.trim_start_matches(perl_compat::unicode::is_perl_space);
                if trimmed.len() < rest.len() {
                    format!("{}:{}", &t[..p], trimmed)
                } else {
                    t.to_string()
                }
            }
            None => t.to_string(),
        })
    };
    if !end.is_empty() {
        let mut s = nfc(&start.join("-"));
        if let Some(t) = time("hour", "minute", "second") {
            s.push_str(&nfc(&t));
        }
        if let Some(t) = tz("timezone") {
            s.push_str(&nfc(&t));
        }
        let mut e = nfc(&end.join("-"));
        if let Some(t) = time("endhour", "endminute", "endsecond") {
            e.push_str(&nfc(&t));
        }
        if let Some(t) = tz("endtimezone") {
            e.push_str(&nfc(&t));
        }
        w.data_element(n("start"), &s, &[]);
        w.data_element(n("end"), &e, &[]);
    } else {
        w.characters(&nfc(&start.join("-")));
        if let Some(t) = time("hour", "minute", "second") {
            w.characters(&nfc(&t));
        }
        if let Some(t) = tz("timezone") {
            w.characters(&nfc(&t));
        }
    }
    w.end_tag();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(schema_file_for_dsn("a/b.bib"), "a/b.rng");
        assert_eq!(schema_file_for_dsn("x.bltxml"), "x.rng");
        assert_eq!(schema_file_for_dsn("noext"), "noext.rng");
        assert_eq!(sprintf_2d("5"), "05");
        assert_eq!(sprintf_2d("XX"), "00");
        assert_eq!(sprintf_2d("-3"), "-03");
        assert_eq!(sprintf_2d("123"), "123");
        assert_eq!(date_monthday(Some("0")), None);
    }
}
