//! The data model (`Biber::DataModel`, DataModel.pm:40-1006): entry types,
//! fields with field types/datatypes/formats, constants, constraints and the
//! sorted helper lists the output stage uses, built from the `.bcf` (and, in
//! tool mode, user) data models. Field and entry type names are case-folded
//! and interned.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::{Map, Value};

use crate::perlish::fc;
use crate::xmlin::{XHash, XVal};

/// An interned field name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FieldId(pub u32);

/// An interned entry type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TypeId(pub u32);

// ---------------------------------------------------------------------------
// The raw (XMLin-shaped) data model, used for merging.

#[derive(Clone, Debug, Default)]
struct RawField {
    content: String,
    fieldtype: Option<String>,
    datatype: Option<String>,
    format: Option<String>,
    nullok: Option<String>,
    skip_output: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct RawEntryType {
    content: String,
    skip_output: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct RawConstant {
    name: Option<String>,
    type_: Option<String>,
    content: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct RawEntryFields {
    /// `None`: no `<entrytype>` key (applies to all types).
    entrytypes: Option<Vec<Option<String>>>,
    fields: Vec<Option<String>>,
}

#[derive(Clone, Debug, Default)]
struct RawConstraint {
    type_: Option<String>,
    datatype: Option<String>,
    rangemin: Option<String>,
    rangemax: Option<String>,
    pattern: Option<String>,
    fields: Vec<Option<String>>,
    fieldxor: Vec<Vec<Option<String>>>,
    fieldor: Vec<Vec<Option<String>>>,
    antecedent: (Option<String>, Vec<Option<String>>),
    consequent: (Option<String>, Vec<Option<String>>),
}

#[derive(Clone, Debug, Default)]
struct RawConstraints {
    entrytypes: Option<Vec<Option<String>>>,
    constraints: Vec<RawConstraint>,
}

#[derive(Clone, Debug, Default)]
struct RawDm {
    constants: Vec<RawConstant>,
    entrytypes: Vec<RawEntryType>,
    fields: Vec<RawField>,
    entryfields: Vec<RawEntryFields>,
    multiscriptfields: Vec<Option<String>>,
    constraints: Vec<RawConstraints>,
}

fn s(h: &XHash, k: &str) -> Option<String> {
    h.str(k).map(str::to_string)
}

fn contents(h: &XHash, key: &str) -> Vec<Option<String>> {
    h.list(key)
        .iter()
        .map(|v| match v {
            XVal::Hash(x) => x.content().map(str::to_string),
            XVal::Str(x) => Some(x.clone()),
            XVal::Array(_) => None,
        })
        .collect()
}

impl RawDm {
    fn from_xval(v: &XVal) -> RawDm {
        let Some(dm) = v.as_hash() else {
            return RawDm::default();
        };
        let sub = |k: &str| dm.hash(k).cloned().unwrap_or_default();
        let constants = sub("constants")
            .list("constant")
            .hashes()
            .map(|c| RawConstant {
                name: s(c, "name"),
                type_: s(c, "type"),
                content: c.content().map(str::to_string),
            })
            .collect();
        let entrytypes = sub("entrytypes")
            .list("entrytype")
            .hashes()
            .map(|e| RawEntryType {
                content: e.content().unwrap_or("").to_string(),
                skip_output: s(e, "skip_output"),
            })
            .collect();
        let fields = sub("fields")
            .list("field")
            .hashes()
            .map(|f| RawField {
                content: f.content().unwrap_or("").to_string(),
                fieldtype: s(f, "fieldtype"),
                datatype: s(f, "datatype"),
                format: s(f, "format"),
                nullok: s(f, "nullok"),
                skip_output: s(f, "skip_output"),
            })
            .collect();
        let entryfields = dm
            .list("entryfields")
            .hashes()
            .map(|ef| RawEntryFields {
                entrytypes: ef.contains("entrytype").then(|| contents(ef, "entrytype")),
                fields: contents(ef, "field"),
            })
            .collect();
        let multiscriptfields = contents(&sub("multiscriptfields"), "field");
        let constraints = dm
            .list("constraints")
            .hashes()
            .map(|cs| RawConstraints {
                entrytypes: cs.contains("entrytype").then(|| contents(cs, "entrytype")),
                constraints: cs
                    .list("constraint")
                    .hashes()
                    .map(|c| {
                        let quant = |k: &str| {
                            let h = c.hash(k).cloned().unwrap_or_default();
                            (s(&h, "quant"), contents(&h, "field"))
                        };
                        RawConstraint {
                            type_: s(c, "type"),
                            datatype: s(c, "datatype"),
                            rangemin: s(c, "rangemin"),
                            rangemax: s(c, "rangemax"),
                            pattern: s(c, "pattern"),
                            fields: contents(c, "field"),
                            fieldxor: c
                                .list("fieldxor")
                                .hashes()
                                .map(|x| contents(x, "field"))
                                .collect(),
                            fieldor: c
                                .list("fieldor")
                                .hashes()
                                .map(|x| contents(x, "field"))
                                .collect(),
                            antecedent: quant("antecedent"),
                            consequent: quant("consequent"),
                        }
                    })
                    .collect(),
            })
            .collect();
        RawDm {
            constants,
            entrytypes,
            fields,
            entryfields,
            multiscriptfields,
            constraints,
        }
    }

    /// Merge a user data model into this default one (DataModel.pm:52-112),
    /// including biber's scoping bugs (BB-084).
    fn merge(&mut self, udm: RawDm) {
        for uc in udm.constants {
            let idx = self.constants.iter().position(|c| {
                fc(c.name.as_deref().unwrap_or("")) == fc(uc.name.as_deref().unwrap_or(""))
            });
            match idx {
                Some(i) => self.constants[i] = uc,
                None => self.constants.push(uc),
            }
        }
        // User constraints are appended to the first default constraints
        // block only (losing their own entrytype scoping).
        if let Some(first) = udm.constraints.into_iter().next() {
            if self.constraints.is_empty() {
                self.constraints.push(RawConstraints::default());
            }
            self.constraints[0].constraints.extend(first.constraints);
        }
        for uef in udm.entryfields {
            match &uef.entrytypes {
                Some(ets) => {
                    let first = ets.first().cloned().flatten().unwrap_or_default();
                    let idx = self.entryfields.iter().position(|ef| {
                        ef.entrytypes
                            .as_ref()
                            .and_then(|e| e.first().cloned().flatten())
                            .is_some_and(|c| {
                                crate::perlish::truthy(Some(&c)) && fc(&c) == fc(&first)
                            })
                    });
                    match idx {
                        Some(i) => self.entryfields[i].fields.extend(uef.fields),
                        None => self.entryfields.push(uef),
                    }
                }
                None => {
                    if let Some(i) = self
                        .entryfields
                        .iter()
                        .position(|ef| ef.entrytypes.is_none())
                    {
                        self.entryfields[i].fields.extend(uef.fields);
                    }
                }
            }
        }
        self.entrytypes.extend(udm.entrytypes);
        for f in udm.fields {
            match self
                .fields
                .iter()
                .position(|d| fc(&d.content) == fc(&f.content))
            {
                Some(i) => self.fields[i] = f,
                None => self.fields.push(f),
            }
        }
        self.multiscriptfields.extend(udm.multiscriptfields);
    }
}

// ---------------------------------------------------------------------------
// The built data model.

/// A field definition (`fieldsbyname`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldDef {
    /// Case-folded name.
    pub name: String,
    /// `field` or `list`.
    pub fieldtype: Option<String>,
    /// literal, name, date, …
    pub datatype: Option<String>,
    /// `xsv` or `default`.
    pub format: String,
    /// `nullok`.
    pub nullok: bool,
    /// `skip_output`.
    pub skipout: bool,
}

/// A mandatory constraint item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mandatory {
    /// A single field.
    Field(String),
    /// Exactly one of these fields.
    Xor(Vec<String>),
    /// At least one of these fields.
    Or(Vec<String>),
}

/// A conditional constraint: if `antecedent_quant` of `antecedent` exist,
/// then `consequent_quant` of `consequent` must.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conditional {
    /// all, one, none.
    pub antecedent_quant: Option<String>,
    /// Antecedent fields.
    pub antecedent: Vec<String>,
    /// all, one, none.
    pub consequent_quant: Option<String>,
    /// Consequent fields.
    pub consequent: Vec<String>,
}

/// A data constraint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataConstraint {
    /// Fields it applies to.
    pub fields: Vec<String>,
    /// isbn, issn, ismn, integer, datepart, pattern, date.
    pub datatype: Option<String>,
    /// `rangemin`.
    pub rangemin: Option<String>,
    /// `rangemax`.
    pub rangemax: Option<String>,
    /// `pattern`.
    pub pattern: Option<String>,
}

/// The constraints of an entry type.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Constraints {
    /// Mandatory constraints.
    pub mandatory: Vec<Mandatory>,
    /// Conditional constraints.
    pub conditional: Vec<Conditional>,
    /// Data constraints.
    pub data: Vec<DataConstraint>,
}

/// An entry type definition (`entrytypesbyname`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EntryTypeDef {
    /// Case-folded name.
    pub name: String,
    /// `skip_output`.
    pub skipout: bool,
    /// Legal fields (as written in `<entryfields>`, not case-folded).
    pub legal_fields: HashSet<String>,
    /// Constraints.
    pub constraints: Constraints,
}

/// The sorted field lists biber precomputes (`$dm->{helpers}`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Helpers {
    /// All name lists.
    pub namelistsall: Vec<String>,
    /// Name lists that are output.
    pub namelists: Vec<String>,
    /// Lists that are output (not names, verbatim or URIs).
    pub lists: Vec<String>,
    /// Plain fields that are output.
    pub fields: Vec<String>,
    /// Date fields.
    pub datefields: Vec<String>,
    /// Date parts.
    pub dateparts: Vec<String>,
    /// xsv-format fields.
    pub xsv: Vec<String>,
    /// Range fields.
    pub ranges: Vec<String>,
    /// URI fields.
    pub uris: Vec<String>,
    /// URI lists.
    pub urils: Vec<String>,
    /// Verbatim and URI fields and lists.
    pub verbs: Vec<String>,
    /// Verbatim and URI fields.
    pub vfields: Vec<String>,
    /// Verbatim and URI lists.
    pub vlists: Vec<String>,
    /// Integer and datepart fields.
    pub integers: Vec<String>,
}

/// The data model.
#[derive(Clone, Debug, Default)]
pub struct DataModel {
    fields: Vec<FieldDef>,
    field_ids: HashMap<String, FieldId>,
    fieldsbytype: BTreeMap<String, BTreeMap<String, BTreeMap<String, Vec<String>>>>,
    fieldsbyfieldtype: BTreeMap<String, Vec<String>>,
    fieldsbydatatype: BTreeMap<String, Vec<String>>,
    fieldsbyformat: BTreeMap<String, Vec<String>>,
    entrytypes: Vec<EntryTypeDef>,
    type_ids: HashMap<String, TypeId>,
    constants: BTreeMap<String, (Option<String>, Option<String>)>,
    foldtoorig: HashMap<String, String>,
    multiscriptfields: Vec<String>,
    /// The helper lists.
    pub helpers: Helpers,
    integer_set: HashSet<String>,
}

/// An error building the data model (`biber_error`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DmError(pub String);

fn remove_from(v: &mut Vec<String>, name: &str) {
    v.retain(|x| x != name);
}

impl DataModel {
    /// `Biber::DataModel->new($dms)`: build from the list of raw data models
    /// (`getblxoption(undef, 'datamodel')`). Returns the model and any fatal
    /// errors (biber reports each with `biber_error`).
    pub fn new(dms: &[XVal], no_default_datamodel: bool) -> (DataModel, Vec<DmError>) {
        let raw0 = dms.first().map(RawDm::from_xval).unwrap_or_default();
        let raw1 = dms.get(1).map(RawDm::from_xval);
        let mut dm = if no_default_datamodel {
            raw1.unwrap_or_default()
        } else {
            let mut d = raw0;
            if let Some(u) = raw1 {
                d.merge(u);
            }
            d
        };
        let mut model = DataModel::default();
        let mut errors = Vec::new();

        for et in &mut dm.entrytypes {
            let f = fc(&et.content);
            model.foldtoorig.insert(f.clone(), et.content.clone());
            et.content = f;
        }
        for fl in &mut dm.fields {
            let f = fc(&fl.content);
            model.foldtoorig.insert(f.clone(), fl.content.clone());
            fl.content = f;
        }
        for d in dm
            .fields
            .iter()
            .filter(|f| f.datatype.as_deref() == Some("date"))
        {
            // m/date$/: `$` also matches before a final newline.
            let c = d.content.strip_suffix('\n').unwrap_or(&d.content);
            if !c.ends_with("date") {
                errors.push(DmError(format!(
                    "Fatal datamodel error: date field '{}' must end with string 'date'",
                    d.content
                )));
            }
        }
        model.multiscriptfields = dm.multiscriptfields.iter().flatten().cloned().collect();

        let mut byname: HashMap<String, usize> = HashMap::new();
        for f in &dm.fields {
            let ft = f.fieldtype.clone().unwrap_or_default();
            let dt = f.datatype.clone().unwrap_or_default();
            if let Some(&pi) = byname.get(&f.content) {
                let prev = model.fields[pi].clone();
                let pft = prev.fieldtype.clone().unwrap_or_default();
                let pdt = prev.datatype.clone().unwrap_or_default();
                let bt = model
                    .fieldsbytype
                    .entry(pft.clone())
                    .or_default()
                    .entry(pdt.clone())
                    .or_default();
                // BB-085: guarded by the *new* field's format.
                if crate::perlish::truthy(f.format.as_deref()) {
                    remove_from(bt.entry(prev.format.clone()).or_default(), &f.content);
                }
                remove_from(bt.entry("*".into()).or_default(), &f.content);
                remove_from(model.fieldsbyfieldtype.entry(pft).or_default(), &f.content);
                remove_from(model.fieldsbydatatype.entry(pdt).or_default(), &f.content);
                remove_from(
                    model.fieldsbyformat.entry(prev.format.clone()).or_default(),
                    &f.content,
                );
            }
            let format = f
                .format
                .clone()
                .filter(|x| crate::perlish::truthy(Some(x)))
                .unwrap_or_else(|| "default".to_string());
            let def = FieldDef {
                name: f.content.clone(),
                fieldtype: f.fieldtype.clone(),
                datatype: f.datatype.clone(),
                format: format.clone(),
                nullok: crate::perlish::truthy(f.nullok.as_deref()),
                skipout: crate::perlish::truthy(f.skip_output.as_deref()),
            };
            match byname.get(&f.content) {
                Some(&i) => model.fields[i] = def,
                None => {
                    byname.insert(f.content.clone(), model.fields.len());
                    model
                        .field_ids
                        .insert(f.content.clone(), FieldId(model.fields.len() as u32));
                    model.fields.push(def);
                }
            }
            let bt = model
                .fieldsbytype
                .entry(ft.clone())
                .or_default()
                .entry(dt.clone())
                .or_default();
            if crate::perlish::truthy(f.format.as_deref()) {
                bt.entry(f.format.clone().unwrap_or_default())
                    .or_default()
                    .push(f.content.clone());
            }
            bt.entry("*".into()).or_default().push(f.content.clone());
            model
                .fieldsbyfieldtype
                .entry(ft)
                .or_default()
                .push(f.content.clone());
            model
                .fieldsbydatatype
                .entry(dt)
                .or_default()
                .push(f.content.clone());
            model
                .fieldsbyformat
                .entry(format)
                .or_default()
                .push(f.content.clone());
        }

        for c in &dm.constants {
            model.constants.insert(
                c.name.clone().unwrap_or_default(),
                (c.type_.clone(), c.content.clone()),
            );
        }

        for et in &dm.entrytypes {
            let es = et.content.clone();
            let idx = match model.type_ids.get(&es) {
                Some(&TypeId(i)) => i as usize,
                None => {
                    let i = model.entrytypes.len();
                    model.type_ids.insert(es.clone(), TypeId(i as u32));
                    model.entrytypes.push(EntryTypeDef {
                        name: es.clone(),
                        ..EntryTypeDef::default()
                    });
                    i
                }
            };
            let def = &mut model.entrytypes[idx];
            if crate::perlish::truthy(et.skip_output.as_deref()) {
                def.skipout = true;
            }
            let applies = |ets: &Option<Vec<Option<String>>>| match ets {
                None => true,
                Some(v) => v.iter().any(|x| x.as_deref().unwrap_or("") == es),
            };
            for ef in &dm.entryfields {
                if applies(&ef.entrytypes) {
                    for f in &ef.fields {
                        def.legal_fields.insert(f.clone().unwrap_or_default());
                    }
                }
            }
            for cd in &dm.constraints {
                if !applies(&cd.entrytypes) {
                    continue;
                }
                for c in &cd.constraints {
                    let flist = |v: &[Option<String>]| -> Vec<String> {
                        v.iter().map(|x| x.clone().unwrap_or_default()).collect()
                    };
                    match c.type_.as_deref() {
                        Some("mandatory") => {
                            for f in &c.fields {
                                let f = f.clone().unwrap_or_default();
                                if !def
                                    .constraints
                                    .mandatory
                                    .iter()
                                    .any(|m| matches!(m, Mandatory::Field(x) if *x == f))
                                {
                                    def.constraints.mandatory.push(Mandatory::Field(f));
                                }
                            }
                            for x in &c.fieldxor {
                                def.constraints.mandatory.push(Mandatory::Xor(flist(x)));
                            }
                            for x in &c.fieldor {
                                def.constraints.mandatory.push(Mandatory::Or(flist(x)));
                            }
                        }
                        Some("conditional") => def.constraints.conditional.push(Conditional {
                            antecedent_quant: c.antecedent.0.clone(),
                            antecedent: flist(&c.antecedent.1),
                            consequent_quant: c.consequent.0.clone(),
                            consequent: flist(&c.consequent.1),
                        }),
                        Some("data") => def.constraints.data.push(DataConstraint {
                            fields: flist(&c.fields),
                            datatype: c.datatype.clone(),
                            rangemin: c.rangemin.clone(),
                            rangemax: c.rangemax.clone(),
                            pattern: c.pattern.clone(),
                        }),
                        _ => {}
                    }
                }
            }
        }

        model.build_helpers();
        (model, errors)
    }

    fn build_helpers(&mut self) {
        let not_skip = |m: &DataModel, f: &String| !m.field_is_skipout(f);
        let namelistsall = self.get_fields_of_type("list", &["name"], None);
        let namelists = namelistsall
            .iter()
            .filter(|f| not_skip(self, f))
            .cloned()
            .collect();
        let lists = self
            .get_fields_of_fieldtype("list")
            .into_iter()
            .filter(|f| {
                !self.field_is_datatype("name", f)
                    && !self.field_is_skipout(f)
                    && !self.field_is_datatype("verbatim", f)
                    && !self.field_is_datatype("uri", f)
            })
            .collect();
        let fields = self
            .get_fields_of_type(
                "field",
                &["entrykey", "key", "integer", "datepart", "literal", "code"],
                None,
            )
            .into_iter()
            .filter(|f| !self.field_is_skipout(f) && self.get_fieldformat(f) != Some("xsv"))
            .collect();
        let filt = |m: &DataModel, v: Vec<String>| -> Vec<String> {
            v.into_iter().filter(|f| not_skip(m, f)).collect()
        };
        let h = Helpers {
            namelistsall,
            namelists,
            lists,
            fields,
            datefields: self.get_fields_of_type("field", &["date"], None),
            dateparts: self.get_fields_of_type("field", &["datepart"], None),
            xsv: filt(self, self.get_fields_of_fieldformat("xsv")),
            ranges: filt(self, self.get_fields_of_datatype(&["range"])),
            uris: filt(self, self.get_fields_of_type("field", &["uri"], None)),
            urils: filt(self, self.get_fields_of_type("list", &["uri"], None)),
            verbs: filt(self, self.get_fields_of_datatype(&["verbatim", "uri"])),
            vfields: filt(
                self,
                self.get_fields_of_type("field", &["verbatim", "uri"], None),
            ),
            vlists: filt(
                self,
                self.get_fields_of_type("list", &["verbatim", "uri"], None),
            ),
            integers: self.get_fields_of_datatype(&["datepart", "integer"]),
        };
        self.integer_set = h.integers.iter().cloned().collect();
        self.helpers = h;
    }

    // ---- lookups ----

    fn def(&self, field: &str) -> Option<&FieldDef> {
        self.field_ids
            .get(field)
            .map(|id| &self.fields[id.0 as usize])
    }

    /// The interned id of a (case-folded) field name.
    pub fn field_id(&self, field: &str) -> Option<FieldId> {
        self.field_ids.get(field).copied()
    }

    /// The field definition for an id.
    pub fn field(&self, id: FieldId) -> &FieldDef {
        &self.fields[id.0 as usize]
    }

    /// The interned id of a (case-folded) entry type.
    pub fn type_id(&self, entrytype: &str) -> Option<TypeId> {
        self.type_ids.get(entrytype).copied()
    }

    /// The entry type definition for an id.
    pub fn entrytype(&self, id: TypeId) -> &EntryTypeDef {
        &self.entrytypes[id.0 as usize]
    }

    /// `get_outcase`: the original case of a folded field/entry type name.
    pub fn get_outcase(&self, s: &str) -> Option<&str> {
        self.foldtoorig.get(s).map(String::as_str)
    }

    /// Constant names.
    pub fn constants(&self) -> Vec<&str> {
        self.constants.keys().map(String::as_str).collect()
    }

    /// `get_constant_type`.
    pub fn get_constant_type(&self, name: &str) -> Option<&str> {
        self.constants.get(name)?.0.as_deref()
    }

    /// `get_constant_value`: list constants are split on `/\s*,\s*/`.
    pub fn get_constant_value(&self, name: &str) -> Vec<String> {
        let Some((t, v)) = self.constants.get(name) else {
            return Vec::new();
        };
        match t.as_deref() {
            Some("list") => crate::perlish::split_comma(v.as_deref().unwrap_or("")),
            Some("string") => vec![v.clone().unwrap_or_default()],
            _ => Vec::new(),
        }
    }

    /// `is_multiscript` is dead and broken in biber (BB-086): always false.
    pub fn is_multiscript(&self, _field: &str) -> bool {
        false
    }

    /// The `<multiscriptfields>` field names.
    pub fn multiscript_fields(&self) -> &[String] {
        &self.multiscriptfields
    }

    /// `fieldtypes`.
    pub fn fieldtypes(&self) -> Vec<&str> {
        self.fieldsbyfieldtype.keys().map(String::as_str).collect()
    }

    /// `datatypes`.
    pub fn datatypes(&self) -> Vec<&str> {
        self.fieldsbydatatype.keys().map(String::as_str).collect()
    }

    /// `is_field`: a known field, ignoring an annotation suffix
    /// (`<field><annotation_marker>[<named_marker><name>]`).
    pub fn is_field(&self, field: &str, annotation_marker: &str, named_marker: &str) -> bool {
        if !annotation_marker.is_empty() {
            // m/^(.+)(?:$ann)(?:$nam.+)?$/ — the greedy (.+) takes the last
            // occurrence of the marker that leaves a valid suffix.
            let mut search_end = field.len();
            while let Some(pos) = field[..search_end].rfind(annotation_marker) {
                if pos > 0 {
                    let rest = &field[pos + annotation_marker.len()..];
                    let rest = rest.strip_suffix('\n').unwrap_or(rest);
                    let ok = rest.is_empty()
                        || (!named_marker.is_empty()
                            && rest.starts_with(named_marker)
                            && rest.len() > named_marker.len()
                            && !rest[named_marker.len()..].contains('\n'));
                    if ok {
                        return self.def(&field[..pos]).is_some();
                    }
                }
                if pos == 0 {
                    break;
                }
                search_end = pos + annotation_marker.len() - 1;
            }
        }
        self.def(field).is_some()
    }

    /// Entry type names.
    pub fn entrytypes(&self) -> Vec<&str> {
        self.entrytypes.iter().map(|e| e.name.as_str()).collect()
    }

    /// `is_entrytype`.
    pub fn is_entrytype(&self, t: &str) -> bool {
        self.type_ids.contains_key(t)
    }

    /// `is_field_for_entrytype`.
    pub fn is_field_for_entrytype(&self, t: &str, field: &str) -> bool {
        self.type_id(t)
            .is_some_and(|id| self.entrytype(id).legal_fields.contains(field))
    }

    /// `entrytype_is_skipout`.
    pub fn entrytype_is_skipout(&self, t: &str) -> bool {
        self.type_id(t).is_some_and(|id| self.entrytype(id).skipout)
    }

    /// `get_fields_of_fieldtype` (sorted).
    pub fn get_fields_of_fieldtype(&self, ft: &str) -> Vec<String> {
        let mut v = self.fieldsbyfieldtype.get(ft).cloned().unwrap_or_default();
        v.sort();
        v
    }

    /// `get_fields_of_fieldformat` (sorted).
    pub fn get_fields_of_fieldformat(&self, fmt: &str) -> Vec<String> {
        let mut v = self.fieldsbyformat.get(fmt).cloned().unwrap_or_default();
        v.sort();
        v
    }

    /// `get_fields_of_datatype` (sorted; several datatypes allowed).
    pub fn get_fields_of_datatype(&self, dts: &[&str]) -> Vec<String> {
        let mut v = Vec::new();
        for dt in dts {
            if let Some(fs) = self.fieldsbydatatype.get(*dt) {
                v.extend(fs.iter().cloned());
            }
        }
        v.sort();
        v
    }

    /// `get_fields_of_type($fieldtype, $datatype(s), $format)` (sorted);
    /// `format` defaults to `*` (any).
    pub fn get_fields_of_type(&self, ft: &str, dts: &[&str], format: Option<&str>) -> Vec<String> {
        let fmt = format.unwrap_or("*");
        let mut v = Vec::new();
        for dt in dts {
            if let Some(fs) = self
                .fieldsbytype
                .get(ft)
                .and_then(|m| m.get(*dt))
                .and_then(|m| m.get(fmt))
            {
                v.extend(fs.iter().cloned());
            }
        }
        v.sort();
        v
    }

    /// `is_fields_of_type`.
    pub fn is_fields_of_type(&self, ft: &str, dt: &str, format: Option<&str>) -> bool {
        let Some(m) = self.fieldsbytype.get(ft).and_then(|m| m.get(dt)) else {
            return false;
        };
        match format.filter(|f| crate::perlish::truthy(Some(f))) {
            Some(f) => m.contains_key(f),
            None => true,
        }
    }

    /// `get_fieldtype`.
    pub fn get_fieldtype(&self, f: &str) -> Option<&str> {
        self.def(f)?.fieldtype.as_deref()
    }

    /// `get_datatype`.
    pub fn get_datatype(&self, f: &str) -> Option<&str> {
        self.def(f)?.datatype.as_deref()
    }

    /// `get_fieldformat`.
    pub fn get_fieldformat(&self, f: &str) -> Option<&str> {
        self.def(f).map(|d| d.format.as_str())
    }

    /// `field_is_fieldtype`.
    pub fn field_is_fieldtype(&self, ft: &str, f: &str) -> bool {
        self.get_fieldtype(f).unwrap_or("") == ft
    }

    /// `field_is_datatype`.
    pub fn field_is_datatype(&self, dt: &str, f: &str) -> bool {
        self.get_datatype(f).unwrap_or("") == dt
    }

    /// `field_is_type`.
    pub fn field_is_type(&self, ft: &str, dt: &str, f: &str) -> bool {
        self.def(f).is_some_and(|d| {
            d.fieldtype.as_deref().unwrap_or("") == ft && d.datatype.as_deref().unwrap_or("") == dt
        })
    }

    /// `field_is_nullok`.
    pub fn field_is_nullok(&self, f: &str) -> bool {
        self.def(f).is_some_and(|d| d.nullok)
    }

    /// `field_is_skipout`.
    pub fn field_is_skipout(&self, f: &str) -> bool {
        self.def(f).is_some_and(|d| d.skipout)
    }

    /// `sortdataschema`: `int` for citeorder/intciteorder/citecount and
    /// integer/datepart fields, else `str`.
    pub fn sortdataschema(&self, f: &str) -> &'static str {
        if matches!(f, "intciteorder" | "citeorder" | "citecount") || self.integer_set.contains(f) {
            "int"
        } else {
            "str"
        }
    }

    /// The constraints of an entry type.
    pub fn constraints(&self, t: &str) -> Option<&Constraints> {
        self.type_id(t).map(|id| &self.entrytype(id).constraints)
    }

    /// JSON dump in the shape of biber's `Biber::DataModel` object (minus
    /// the broken `multiscriptfields` and the `sortdataschema` closure).
    pub fn to_json(&self) -> Value {
        let strs = |v: &[String]| Value::Array(v.iter().map(|x| x.clone().into()).collect());
        let opt = |v: &Option<String>| v.clone().map_or(Value::Null, Value::String);
        let mut m = Map::new();
        let mut byname = Map::new();
        for f in &self.fields {
            let mut o = Map::new();
            o.insert("fieldtype".into(), opt(&f.fieldtype));
            o.insert("datatype".into(), opt(&f.datatype));
            o.insert("format".into(), f.format.clone().into());
            if f.nullok {
                o.insert("nullok".into(), "1".into());
            }
            if f.skipout {
                o.insert("skipout".into(), "1".into());
            }
            byname.insert(f.name.clone(), Value::Object(o));
        }
        m.insert("fieldsbyname".into(), Value::Object(byname));
        let mut bt = Map::new();
        for (ft, dm) in &self.fieldsbytype {
            let mut a = Map::new();
            for (dt, fm) in dm {
                let mut b = Map::new();
                for (fmt, v) in fm {
                    b.insert(fmt.clone(), strs(v));
                }
                a.insert(dt.clone(), Value::Object(b));
            }
            bt.insert(ft.clone(), Value::Object(a));
        }
        m.insert("fieldsbytype".into(), Value::Object(bt));
        for (name, src) in [
            ("fieldsbyfieldtype", &self.fieldsbyfieldtype),
            ("fieldsbydatatype", &self.fieldsbydatatype),
            ("fieldsbyformat", &self.fieldsbyformat),
        ] {
            m.insert(
                name.into(),
                Value::Object(src.iter().map(|(k, v)| (k.clone(), strs(v))).collect()),
            );
        }
        let mut consts = Map::new();
        for (k, (t, v)) in &self.constants {
            let mut o = Map::new();
            o.insert("type".into(), opt(t));
            o.insert("value".into(), opt(v));
            consts.insert(k.clone(), Value::Object(o));
        }
        m.insert("constants".into(), Value::Object(consts));
        let mut ets = Map::new();
        for et in &self.entrytypes {
            let mut o = Map::new();
            if et.skipout {
                o.insert("skipout".into(), "1".into());
            }
            if !et.legal_fields.is_empty() {
                let mut lf: Vec<&String> = et.legal_fields.iter().collect();
                lf.sort();
                o.insert(
                    "legal_fields".into(),
                    Value::Object(
                        lf.into_iter()
                            .map(|f| (f.clone(), Value::from("1")))
                            .collect(),
                    ),
                );
            }
            let c = &et.constraints;
            if !c.mandatory.is_empty() || !c.conditional.is_empty() || !c.data.is_empty() {
                let mut co = Map::new();
                if !c.mandatory.is_empty() {
                    co.insert(
                        "mandatory".into(),
                        Value::Array(
                            c.mandatory
                                .iter()
                                .map(|x| match x {
                                    Mandatory::Field(f) => f.clone().into(),
                                    Mandatory::Xor(v) | Mandatory::Or(v) => {
                                        let tag = if matches!(x, Mandatory::Xor(_)) {
                                            "XOR"
                                        } else {
                                            "OR"
                                        };
                                        let mut a = vec![Value::from(tag)];
                                        a.extend(v.iter().map(|s| Value::from(s.clone())));
                                        Value::Array(a)
                                    }
                                })
                                .collect(),
                        ),
                    );
                }
                if !c.conditional.is_empty() {
                    co.insert(
                        "conditional".into(),
                        Value::Array(
                            c.conditional
                                .iter()
                                .map(|x| {
                                    Value::Array(vec![
                                        opt(&x.antecedent_quant),
                                        strs(&x.antecedent),
                                        opt(&x.consequent_quant),
                                        strs(&x.consequent),
                                    ])
                                })
                                .collect(),
                        ),
                    );
                }
                if !c.data.is_empty() {
                    co.insert(
                        "data".into(),
                        Value::Array(
                            c.data
                                .iter()
                                .map(|d| {
                                    let mut o = Map::new();
                                    o.insert("fields".into(), strs(&d.fields));
                                    o.insert("datatype".into(), opt(&d.datatype));
                                    o.insert("rangemin".into(), opt(&d.rangemin));
                                    o.insert("rangemax".into(), opt(&d.rangemax));
                                    o.insert("pattern".into(), opt(&d.pattern));
                                    Value::Object(o)
                                })
                                .collect(),
                        ),
                    );
                }
                o.insert("constraints".into(), Value::Object(co));
            }
            ets.insert(et.name.clone(), Value::Object(o));
        }
        m.insert("entrytypesbyname".into(), Value::Object(ets));
        let mut casemap = Map::new();
        let mut keys: Vec<&String> = self.foldtoorig.keys().collect();
        keys.sort();
        for k in keys {
            casemap.insert(k.clone(), self.foldtoorig[k].clone().into());
        }
        let mut cm = Map::new();
        cm.insert("foldtoorig".into(), Value::Object(casemap));
        m.insert("casemap".into(), Value::Object(cm));
        let h = &self.helpers;
        let mut hm = Map::new();
        for (k, v) in [
            ("namelistsall", &h.namelistsall),
            ("namelists", &h.namelists),
            ("lists", &h.lists),
            ("fields", &h.fields),
            ("datefields", &h.datefields),
            ("dateparts", &h.dateparts),
            ("xsv", &h.xsv),
            ("ranges", &h.ranges),
            ("uris", &h.uris),
            ("urils", &h.urils),
            ("verbs", &h.verbs),
            ("vfields", &h.vfields),
            ("vlists", &h.vlists),
            ("integers", &h.integers),
        ] {
            hm.insert(k.into(), strs(v));
        }
        m.insert("helpers".into(), Value::Object(hm));
        Value::Object(m)
    }
}

// ---------------------------------------------------------------------------
// --validate-datamodel

/// A field value as the datatype checks see it.
#[derive(Clone, Debug)]
pub enum FieldValue<'a> {
    /// A plain string.
    Str(&'a str),
    /// A list or range (an array reference in biber).
    Array(Vec<&'a str>),
    /// A name list (`Biber::Entry::Names`).
    Names,
    /// Some other reference.
    Other,
}

/// The entry interface the validator needs.
pub trait ValidateEntry {
    /// The (case-folded) entry type.
    fn entrytype(&self) -> String;
    /// The citation key.
    fn citekey(&self) -> String;
    /// `field_exists`.
    fn field_exists(&self, f: &str) -> bool;
    /// `get_field` (None if absent).
    fn get_field(&self, f: &str) -> Option<FieldValue<'_>>;
    /// Perl truthiness of `get_field($f)`.
    fn field_truthy(&self, f: &str) -> bool {
        match self.get_field(f) {
            None => false,
            Some(FieldValue::Str(s)) => crate::perlish::truthy(Some(s)),
            Some(_) => true,
        }
    }
    /// `del_field`.
    fn del_field(&mut self, f: &str);
    /// `set_field('entrytype', …)`.
    fn set_entrytype(&mut self, t: &str);
    /// `datafields`: the data fields present.
    fn datafields(&self) -> Vec<String>;
    /// `fields`: all fields present.
    fn fields(&self) -> Vec<String>;
}

/// Checks that depend on other crates (ISBN/ISSN/ISMN validity, Perl's
/// `looks_like_number(num(…))`, the `imatch` regex engine).
pub trait DatatypeChecks {
    /// The `isbn` data constraint (`Business::ISBN->new` succeeds).
    fn isbn_ok(&self, v: &str) -> bool;
    /// The `issn` data constraint.
    fn issn_ok(&self, v: &str) -> bool;
    /// The `ismn` data constraint.
    fn ismn_ok(&self, v: &str) -> bool;
    /// `looks_like_number(num($v =~ s/^-//r))`.
    fn is_ucd_number(&self, v: &str) -> bool {
        let v = v.strip_prefix('-').unwrap_or(v);
        !v.is_empty() && v.chars().all(|c| c.is_ascii_digit())
    }
    /// `imatch($value, $pattern)` (case-insensitive, unanchored).
    fn imatch(&self, v: &str, pattern: &str) -> bool;
}

/// The year-division names (`%YEARDIVISIONS` values).
pub const YEARDIVISIONS: &[&str] = &[
    "spring", "summer", "autumn", "winter", "springN", "summerN", "autumnN", "winterN", "springS",
    "summerS", "autumnS", "winterS", "Q1", "Q2", "Q3", "Q4", "QD1", "QD2", "QD3", "S1", "S2",
];

impl DataModel {
    /// `check_mandatory_constraints`: warnings for one entry (may delete
    /// surplus XOR fields). `ds` is the entry's data source name.
    pub fn check_mandatory_constraints(&self, be: &mut dyn ValidateEntry, ds: &str) -> Vec<String> {
        let et = be.entrytype();
        let key = be.citekey();
        let mut w = Vec::new();
        let Some(c) = self.constraints(&et) else {
            return w;
        };
        for m in c.mandatory.clone() {
            match m {
                Mandatory::Xor(fs) => {
                    let mut flag = false;
                    let mut xorflag = false;
                    for of in &fs {
                        if be.field_exists(of) && !(of == "date" && be.field_truthy("datesplit")) {
                            if xorflag {
                                w.push(format!(
                                    "Datamodel: {et} entry '{key}' ({ds}): Mandatory fields - only one of '{}' must be defined - ignoring field '{of}'",
                                    fs.join(", ")
                                ));
                                be.del_field(of);
                            }
                            flag = true;
                            xorflag = true;
                        }
                    }
                    if !flag {
                        w.push(format!(
                            "Datamodel: {et} entry '{key}' ({ds}): Missing mandatory field - one of '{}' must be defined",
                            fs.join(", ")
                        ));
                    }
                }
                Mandatory::Or(fs) => {
                    if !fs.iter().any(|f| be.field_exists(f)) {
                        w.push(format!(
                            "Datamodel: {et} entry '{key}' ({ds}): Missing mandatory field - one of '{}' must be defined",
                            fs.join(", ")
                        ));
                    }
                }
                Mandatory::Field(f) => {
                    if !be.field_exists(&f) {
                        w.push(format!(
                            "Datamodel: {et} entry '{key}' ({ds}): Missing mandatory field '{f}'"
                        ));
                    }
                }
            }
        }
        w
    }

    /// `check_conditional_constraints`.
    pub fn check_conditional_constraints(
        &self,
        be: &mut dyn ValidateEntry,
        ds: &str,
    ) -> Vec<String> {
        let et = be.entrytype();
        let key = be.citekey();
        let mut w = Vec::new();
        let Some(c) = self.constraints(&et) else {
            return w;
        };
        for c in c.conditional.clone() {
            let aq = c.antecedent_quant.as_deref().unwrap_or("");
            let cq = c.consequent_quant.as_deref().unwrap_or("");
            let actual_afs: Vec<&String> =
                c.antecedent.iter().filter(|f| be.field_exists(f)).collect();
            match aq {
                "all" if c.antecedent.len() != actual_afs.len() => continue,
                "none" if !actual_afs.is_empty() => continue,
                "one" if actual_afs.is_empty() => continue,
                _ => {}
            }
            let actual_cfs: Vec<String> = c
                .consequent
                .iter()
                .filter(|f| be.field_exists(f))
                .cloned()
                .collect();
            let afs = c.antecedent.join(", ");
            match cq {
                "all" if c.consequent.len() != actual_cfs.len() => w.push(format!(
                    "Datamodel: {et} entry '{key}' ({ds}): Constraint violation - {cq} of fields ({}) must exist when {aq} of fields ({afs}) exist",
                    c.consequent.join(", ")
                )),
                "none" if !actual_cfs.is_empty() => {
                    w.push(format!(
                        "Datamodel: {et} entry '{key}' ({ds}): Constraint violation - {cq} of fields ({}) must exist when {aq} of fields ({afs}) exist. Ignoring them.",
                        actual_cfs.join(", ")
                    ));
                    for f in &actual_cfs {
                        be.del_field(f);
                    }
                }
                "one" if actual_cfs.is_empty() => w.push(format!(
                    "Datamodel: {et} entry '{key}' ({ds}): Constraint violation - {cq} of fields ({}) must exist when {aq} of fields ({afs}) exist",
                    c.consequent.join(", ")
                )),
                _ => {}
            }
        }
        w
    }

    /// `check_data_constraints`.
    pub fn check_data_constraints(
        &self,
        be: &mut dyn ValidateEntry,
        ds: &str,
        checks: &dyn DatatypeChecks,
    ) -> Vec<String> {
        let et = be.entrytype();
        let key = be.citekey();
        let mut w = Vec::new();
        let Some(c) = self.constraints(&et) else {
            return w;
        };
        for c in c.data.clone() {
            let dt = c.datatype.as_deref().unwrap_or("");
            match dt {
                "isbn" | "issn" | "ismn" => {
                    for f in &c.fields {
                        if !be.field_truthy(f) {
                            continue;
                        }
                        let vals: Vec<String> = match be.get_field(f) {
                            Some(FieldValue::Str(s)) => vec![s.to_string()],
                            Some(FieldValue::Array(a)) => a.iter().map(|s| s.to_string()).collect(),
                            _ => Vec::new(),
                        };
                        for v in vals {
                            let ok = match dt {
                                "isbn" => checks.isbn_ok(&v),
                                "issn" => checks.issn_ok(&v),
                                _ => checks.ismn_ok(&v),
                            };
                            if !ok {
                                w.push(format!(
                                    "Datamodel: {et} entry '{key}' ({ds}): Invalid {} in value of field '{f}'",
                                    dt.to_ascii_uppercase()
                                ));
                            }
                        }
                    }
                }
                "integer" | "datepart" => {
                    for f in &c.fields {
                        let Some(FieldValue::Str(fv)) = be.get_field(f) else {
                            continue;
                        };
                        if !crate::perlish::truthy(Some(fv)) {
                            continue;
                        }
                        let fvn = crate::perlish::numify(fv);
                        // BB-058: a range bound of 0 is ignored.
                        if let Some(min) = c
                            .rangemin
                            .as_deref()
                            .filter(|m| crate::perlish::truthy(Some(m)))
                            && fvn < crate::perlish::numify(min)
                        {
                            w.push(format!(
                                "Datamodel: {et} entry '{key}' ({ds}): Invalid value of field '{f}' must be '>={min}' - ignoring field"
                            ));
                            be.del_field(f);
                            continue;
                        }
                        if let Some(max) = c
                            .rangemax
                            .as_deref()
                            .filter(|m| crate::perlish::truthy(Some(m)))
                            && fvn > crate::perlish::numify(max)
                        {
                            w.push(format!(
                                "Datamodel: {et} entry '{key}' ({ds}): Invalid value of field '{f}' must be '<={max}' - ignoring field"
                            ));
                            be.del_field(f);
                            continue;
                        }
                    }
                }
                "pattern" => {
                    let patt = c
                        .pattern
                        .clone()
                        .filter(|p| crate::perlish::truthy(Some(p)));
                    if patt.is_none() {
                        w.push("Datamodel: Pattern constraint has no pattern!".to_string());
                    }
                    for f in &c.fields {
                        let Some(FieldValue::Str(fv)) = be.get_field(f) else {
                            continue;
                        };
                        if !crate::perlish::truthy(Some(fv)) {
                            continue;
                        }
                        if !checks.imatch(fv, patt.as_deref().unwrap_or("")) {
                            w.push(format!(
                                "Datamodel: {et} entry '{key}' ({ds}): Invalid value (pattern match fails) for field '{f}'"
                            ));
                        }
                    }
                }
                _ => {}
            }
        }
        w
    }

    /// `check_datatypes`: deletes fields whose value doesn't fit the
    /// datatype.
    pub fn check_datatypes(
        &self,
        be: &mut dyn ValidateEntry,
        ds: &str,
        checks: &dyn DatatypeChecks,
    ) -> Vec<String> {
        let et = be.entrytype();
        let key = be.citekey();
        let mut w = Vec::new();
        for f in be.fields() {
            let Some(fdt) = self.get_datatype(&f).map(str::to_string) else {
                continue;
            };
            let fft = self.get_fieldtype(&f).unwrap_or("");
            let ffmt = self.get_fieldformat(&f).unwrap_or("");
            let Some(fv) = be.get_field(&f) else { continue };
            let as_list = (fft == "list" && fdt != "name") || ffmt == "xsv";
            if self.field_is_nullok(&f) && matches!(fv, FieldValue::Str("")) {
                continue;
            }
            let ok = if as_list {
                true // DM_DATATYPES{list} always returns 1
            } else {
                match fdt.as_str() {
                    "integer" => match &fv {
                        FieldValue::Str(s) => checks.is_ucd_number(s),
                        _ => false,
                    },
                    "name" => matches!(fv, FieldValue::Names),
                    "range" => matches!(fv, FieldValue::Array(_)),
                    "datepart" => match &fv {
                        FieldValue::Str(v) => {
                            if f.ends_with("timezone") {
                                *v == "Z" || timezone_ok(v)
                            } else if f.ends_with("season") {
                                ["winter", "spring", "summer", "autumn"]
                                    .iter()
                                    .any(|x| v.contains(x))
                            } else if f.ends_with("yeardivision") {
                                YEARDIVISIONS.contains(v)
                            } else {
                                checks.is_ucd_number(v)
                            }
                        }
                        _ => true,
                    },
                    _ => matches!(fv, FieldValue::Str(_)),
                }
            };
            if !ok {
                w.push(format!(
                    "Datamodel: {et} entry '{key}' ({ds}): Invalid value of field '{f}' must be datatype '{fdt}' - ignoring field"
                ));
                be.del_field(&f);
            }
        }
        w
    }

    /// `validate_datamodel` for one entry (Biber.pm:1702-1765): returns the
    /// messages in biber's order; the caller reports each with `biber_warn`
    /// (or `biber_error` with `--dieondatamodel`).
    pub fn validate_entry(
        &self,
        be: &mut dyn ValidateEntry,
        ds: &str,
        checks: &dyn DatatypeChecks,
    ) -> Vec<String> {
        let mut msgs = Vec::new();
        let bee = be.entrytype();
        let citekey = be.citekey();
        let mut et = bee.clone();
        if !self.is_entrytype(&et) {
            msgs.push(format!(
                "Datamodel: {bee} entry '{citekey}' ({ds}): Invalid entry type '{bee}' - defaulting to 'misc'"
            ));
            be.set_entrytype("misc");
            et = "misc".to_string();
        }
        if et != "xdata" && et != "set" {
            for ef in be.datafields() {
                if !self.is_field_for_entrytype(&et, &ef) {
                    msgs.push(format!(
                        "Datamodel: {bee} entry '{citekey}' ({ds}): Invalid field '{ef}' for entrytype '{et}'"
                    ));
                }
            }
        }
        msgs.extend(self.check_mandatory_constraints(be, ds));
        msgs.extend(self.check_conditional_constraints(be, ds));
        msgs.extend(self.check_datatypes(be, ds, checks));
        msgs.extend(self.check_data_constraints(be, ds, checks));
        msgs
    }
}

fn timezone_ok(v: &str) -> bool {
    // m|^[+-]\d\d(?:\\bibtzminsep\s)?(?:\d\d)?$|
    let b = v.strip_suffix('\n').unwrap_or(v);
    let mut rest = match b.strip_prefix(['+', '-']) {
        Some(r) => r,
        None => return false,
    };
    let two_digits = |s: &str| s.len() >= 2 && s.as_bytes()[..2].iter().all(u8::is_ascii_digit);
    if !two_digits(rest) {
        return false;
    }
    rest = &rest[2..];
    if let Some(r) = rest.strip_prefix("\\bibtzminsep")
        && let Some(c) = r.chars().next()
        && c.is_whitespace()
    {
        rest = &r[c.len_utf8()..];
    }
    rest.is_empty() || (rest.len() == 2 && two_digits(rest))
}
