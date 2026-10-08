//! `Biber::validate_datamodel` (Biber.pm:1702) and the data model checks it
//! uses (DataModel.pm:730-1000, `%DM_DATATYPES` in Constants.pm:122).

use super::{SharedLists, del_field, entry_key, warn_entry};
use crate::datamodel::{DataModel, Mandatory};
use crate::log::Fatal;
use crate::model::{Entry, SectionState, Value};
use crate::pipeline::Ctx;
use crate::pipeline::fetch::SecExt;
use crate::quirks::Quirk;
use crate::{biber_error, info};
use perl_compat::perl_truthy;

/// The ISO 8601-2 year divisions (`%YEARDIVISIONS` values).
const YEARDIVISIONS: [&str; 21] = [
    "spring", "summer", "autumn", "winter", "springN", "summerN", "autumnN", "winterN", "springS",
    "summerS", "autumnS", "winterS", "Q1", "Q2", "Q3", "Q4", "QD1", "QD2", "QD3", "S1", "S2",
];

/// `Biber::validate_datamodel` (Biber.pm:1702).
pub fn validate_datamodel(ctx: &mut Ctx, st: &mut SectionState) -> Result<(), Fatal> {
    if !ctx.cfg.biber.validate_datamodel() {
        return Ok(());
    }
    info!(ctx.log, "Datamodel validation starting");
    let mut sh = SharedLists::load(st);
    let die = ctx.cfg.biber.dieondatamodel();
    let dm = ctx.dm;
    for citekey in st.section.citekeys.clone() {
        let Some(key) = entry_key(&st.section, &citekey) else {
            continue;
        };
        let bee = st.section.entries[&key].entrytype().to_string();
        let ck = st.section.entries[&key].citekey().to_string();
        let ds = st.section.get_keytods(&ck).unwrap_or_default();
        let mut msgs: Vec<String> = Vec::new();
        let mut et = bee.clone();
        if !dm.is_entrytype(&et) {
            msgs.push(format!(
                "Datamodel: {bee} entry '{ck}' ({ds}): Invalid entry type '{bee}' - defaulting to 'misc'"
            ));
            st.section
                .entries
                .get_mut(&key)
                .expect("entry")
                .set_field("entrytype", Some(Value::Str("misc".into())));
            et = "misc".into();
        }
        if et != "xdata" && et != "set" {
            for ef in st.section.entries[&key].datafields.keys() {
                if !dm.is_field_for_entrytype(&et, ef) {
                    msgs.push(format!(
                        "Datamodel: {bee} entry '{ck}' ({ds}): Invalid field '{ef}' for entrytype '{et}'"
                    ));
                }
            }
        }
        // The checks read the (possibly reset) entry type and delete
        // offending fields as they go; report each batch in order.
        let hd = Hdr {
            et: st.section.entries[&key].entrytype().to_string(),
            key: ck.clone(),
            ds,
        };
        msgs.extend(check_mandatory(dm, st, &mut sh, &key, &hd));
        msgs.extend(check_conditional(dm, st, &mut sh, &key, &hd));
        msgs.extend(check_datatypes(dm, st, &mut sh, &key, &hd));
        msgs.extend(check_data_constraints(dm, st, &mut sh, &key, &hd));
        for m in msgs {
            if die {
                // BB-067: the entry lands in biber_error's `$nodie` slot.
                if ctx.cfg.quirks.on(Quirk::DieondatamodelNoExit) {
                    ctx.log.biber_error_nodie(crate::loc!(), &m);
                } else {
                    let r = biber_error!(ctx.log, "{m}");
                    if r.is_err() {
                        sh.store(st);
                        return r;
                    }
                }
            } else {
                warn_entry(ctx, st, &key, m);
            }
        }
    }
    sh.store(st);
    info!(ctx.log, "Datamodel validation complete");
    Ok(())
}

struct Hdr {
    et: String,
    key: String,
    ds: String,
}

impl Hdr {
    fn p(&self) -> String {
        format!("Datamodel: {} entry '{}' ({})", self.et, self.key, self.ds)
    }
}

/// `check_mandatory_constraints` (DataModel.pm:730).
fn check_mandatory(
    dm: &DataModel,
    st: &mut SectionState,
    sh: &mut SharedLists,
    key: &str,
    h: &Hdr,
) -> Vec<String> {
    let mut w = Vec::new();
    let Some(c) = dm.constraints(&h.et) else {
        return w;
    };
    for m in &c.mandatory {
        match m {
            Mandatory::Xor(fs) => {
                let mut flag = false;
                for of in fs {
                    let e = &st.section.entries[key];
                    if e.field_exists(of)
                        && !(of == "date" && super::truthy(e.get_field("datesplit")))
                    {
                        if flag {
                            w.push(format!(
                                "{}: Mandatory fields - only one of '{}' must be defined - ignoring field '{of}'",
                                h.p(),
                                fs.join(", ")
                            ));
                            del_field(st, sh, key, of);
                        }
                        flag = true;
                    }
                }
                if !flag {
                    w.push(format!(
                        "{}: Missing mandatory field - one of '{}' must be defined",
                        h.p(),
                        fs.join(", ")
                    ));
                }
            }
            Mandatory::Or(fs) => {
                let e = &st.section.entries[key];
                if !fs.iter().any(|f| e.field_exists(f)) {
                    w.push(format!(
                        "{}: Missing mandatory field - one of '{}' must be defined",
                        h.p(),
                        fs.join(", ")
                    ));
                }
            }
            Mandatory::Field(f) => {
                if !st.section.entries[key].field_exists(f) {
                    w.push(format!("{}: Missing mandatory field '{f}'", h.p()));
                }
            }
        }
    }
    w
}

/// `check_conditional_constraints` (DataModel.pm:795).
fn check_conditional(
    dm: &DataModel,
    st: &mut SectionState,
    sh: &mut SharedLists,
    key: &str,
    h: &Hdr,
) -> Vec<String> {
    let mut w = Vec::new();
    let Some(c) = dm.constraints(&h.et) else {
        return w;
    };
    for c in &c.conditional {
        let aq = c.antecedent_quant.as_deref().unwrap_or("");
        let cq = c.consequent_quant.as_deref().unwrap_or("");
        let e = &st.section.entries[key];
        let afs: Vec<&String> = c.antecedent.iter().filter(|f| e.field_exists(f)).collect();
        let sat = match aq {
            "all" => afs.len() == c.antecedent.len(),
            "none" => afs.is_empty(),
            "one" => !afs.is_empty(),
            _ => true,
        };
        if !sat {
            continue;
        }
        let cfs: Vec<String> = c
            .consequent
            .iter()
            .filter(|f| e.field_exists(f))
            .cloned()
            .collect();
        let ants = c.antecedent.join(", ");
        match cq {
            "all" if cfs.len() != c.consequent.len() => w.push(format!(
                "{}: Constraint violation - {cq} of fields ({}) must exist when {aq} of fields ({ants}) exist",
                h.p(),
                c.consequent.join(", ")
            )),
            "none" if !cfs.is_empty() => {
                w.push(format!(
                    "{}: Constraint violation - {cq} of fields ({}) must exist when {aq} of fields ({ants}) exist. Ignoring them.",
                    h.p(),
                    cfs.join(", ")
                ));
                for f in &cfs {
                    del_field(st, sh, key, f);
                }
            }
            "one" if cfs.is_empty() => w.push(format!(
                "{}: Constraint violation - {cq} of fields ({}) must exist when {aq} of fields ({ants}) exist",
                h.p(),
                c.consequent.join(", ")
            )),
            _ => {}
        }
    }
    w
}

/// Perl's string form of a field value (`"$v"`): references never equal
/// a plain string.
fn as_scalar(v: Option<&Value>) -> Option<&str> {
    match v {
        None => Some(""),
        Some(Value::Str(s)) => Some(s),
        Some(_) => None,
    }
}

/// `looks_like_number(num($v =~ s/^-//r))`.
fn is_integer(v: &str) -> bool {
    perl_compat::unicode::ucd_num(v.strip_prefix('-').unwrap_or(v)).is_some()
}

/// `$DM_DATATYPES{$dt}->($v, $f)`.
fn datatype_ok(dt: &str, v: Option<&Value>, f: &str) -> bool {
    match dt {
        "integer" => as_scalar(v).is_some_and(is_integer),
        "name" => matches!(v, Some(Value::Names(_))),
        "range" | "list" => matches!(
            v,
            Some(Value::List(_) | Value::Ranges(_) | Value::Other(serde_json::Value::Array(_)))
        ),
        "datepart" => {
            let Some(v) = as_scalar(v) else {
                return false;
            };
            if f.ends_with("timezone") {
                v == "Z" || tz_ok(v)
            } else if f.ends_with("season") {
                ["winter", "spring", "summer", "autumn"]
                    .iter()
                    .any(|s| v.contains(s))
            } else if f.ends_with("yeardivision") {
                YEARDIVISIONS.contains(&v)
            } else {
                is_integer(v)
            }
        }
        "isbn" => as_scalar(v).is_some_and(|v| perl_compat::isbn::Isbn::new(v).is_some()),
        "issn" => as_scalar(v)
            .is_some_and(|v| perl_compat::isbn::Issn::new(v).is_some_and(|i| i.is_valid())),
        "ismn" => as_scalar(v).is_some_and(|v| {
            perl_compat::isbn::Ismn::new(v).is_some_and(|i| i.is_valid_perl_true())
        }),
        _ => as_scalar(v).is_some(),
    }
}

/// `m|^[+-]\d\d(?:\\bibtzminsep\s)?(?:\d\d)?$|`
fn tz_ok(v: &str) -> bool {
    let d = perl_compat::unicode::is_perl_digit;
    let c: Vec<char> = v.chars().collect();
    if c.len() < 3 || !matches!(c[0], '+' | '-') || !d(c[1]) || !d(c[2]) {
        return false;
    }
    let mut rest = &c[3..];
    let marker: Vec<char> = "\\bibtzminsep".chars().collect();
    if rest.len() > marker.len()
        && rest[..marker.len()] == marker[..]
        && perl_compat::unicode::is_perl_space(rest[marker.len()])
    {
        rest = &rest[marker.len() + 1..];
    }
    // `$` also matches before a final newline.
    let rest: &[char] = match rest.last() {
        Some('\n') => &rest[..rest.len() - 1],
        _ => rest,
    };
    rest.is_empty() || (rest.len() == 2 && d(rest[0]) && d(rest[1]))
}

/// `check_datatypes` (DataModel.pm:965).
fn check_datatypes(
    dm: &DataModel,
    st: &mut SectionState,
    sh: &mut SharedLists,
    key: &str,
    h: &Hdr,
) -> Vec<String> {
    let mut w = Vec::new();
    let e: &Entry = &st.section.entries[key];
    let mut fields: Vec<String> = e
        .datafields
        .keys()
        .chain(e.derivedfields.keys())
        .cloned()
        .collect();
    fields.sort();
    fields.dedup();
    for f in fields {
        let Some(fdt) = dm.get_datatype(&f) else {
            continue;
        };
        let fft = dm.get_fieldtype(&f).unwrap_or("");
        let ffmt = dm.get_fieldformat(&f).unwrap_or("");
        let known = [
            "integer", "name", "range", "list", "datepart", "isbn", "issn", "ismn",
        ];
        let mut dt = if known.contains(&fdt) { fdt } else { "default" };
        if (fft == "list" && fdt != "name") || ffmt == "xsv" {
            dt = "list";
        }
        let fv = st.section.entries[key].get_field(&f).cloned();
        if dm.field_is_nullok(&f) && as_scalar(fv.as_ref()) == Some("") {
            continue;
        }
        if !datatype_ok(dt, fv.as_ref(), &f) {
            w.push(format!(
                "{}: Invalid value of field '{f}' must be datatype '{fdt}' - ignoring field",
                h.p()
            ));
            del_field(st, sh, key, &f);
        }
    }
    w
}

/// `check_data_constraints` (DataModel.pm:860).
fn check_data_constraints(
    dm: &DataModel,
    st: &mut SectionState,
    sh: &mut SharedLists,
    key: &str,
    h: &Hdr,
) -> Vec<String> {
    let mut w = Vec::new();
    let Some(c) = dm.constraints(&h.et) else {
        return w;
    };
    // `if (my $fv = $be->get_field($f))`
    let get = |st: &SectionState, f: &str| {
        st.section.entries[key]
            .get_field(f)
            .filter(|v| super::truthy(Some(v)))
            .cloned()
    };
    for c in &c.data {
        let dt = c.datatype.as_deref().unwrap_or("");
        match dt {
            "isbn" | "issn" | "ismn" => {
                let what = dt.to_uppercase();
                for f in &c.fields {
                    let Some(fv) = get(st, f) else { continue };
                    let vals: Vec<Option<Value>> = if dm.get_fieldtype(f) == Some("list") {
                        match &fv {
                            Value::List(l) => l.iter().map(|x| Some(Value::Str(x.clone()))).collect(),
                            _ => vec![Some(fv.clone())],
                        }
                    } else {
                        vec![Some(fv.clone())]
                    };
                    for v in vals {
                        if !datatype_ok(dt, v.as_ref(), f) {
                            w.push(format!("{}: Invalid {what} in value of field '{f}'", h.p()));
                        }
                    }
                }
            }
            "integer" | "datepart" => {
                for f in &c.fields {
                    let Some(fv) = get(st, f) else { continue };
                    let n = match &fv {
                        Value::Str(s) => perl_compat::truth::numify(s),
                        _ => 0.0,
                    };
                    if let Some(min) = c.rangemin.as_deref().filter(|m| perl_truthy(m))
                        && n < perl_compat::truth::numify(min)
                    {
                        w.push(format!(
                            "{}: Invalid value of field '{f}' must be '>={min}' - ignoring field",
                            h.p()
                        ));
                        del_field(st, sh, key, f);
                        continue;
                    }
                    if let Some(max) = c.rangemax.as_deref().filter(|m| perl_truthy(m))
                        && n > perl_compat::truth::numify(max)
                    {
                        w.push(format!(
                            "{}: Invalid value of field '{f}' must be '<={max}' - ignoring field",
                            h.p()
                        ));
                        del_field(st, sh, key, f);
                        continue;
                    }
                }
            }
            "pattern" => {
                let patt = c.pattern.as_deref().filter(|p| perl_truthy(p));
                if patt.is_none() {
                    w.push("Datamodel: Pattern constraint has no pattern!".to_string());
                }
                for f in &c.fields {
                    let Some(fv) = get(st, f) else { continue };
                    let ok = match (patt, &fv) {
                        (Some(p), Value::Str(s)) => perl_compat::regex::PerlRegex::cached(p, false)
                            .ok()
                            .and_then(|r| r.is_match(s).ok())
                            .unwrap_or(false),
                        _ => false,
                    };
                    if !ok {
                        w.push(format!(
                            "{}: Invalid value (pattern match fails) for field '{f}'",
                            h.p()
                        ));
                    }
                }
            }
            _ => {}
        }
    }
    w
}
