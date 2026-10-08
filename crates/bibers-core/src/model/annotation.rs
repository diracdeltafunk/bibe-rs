//! Data annotations (`Biber::Annotation`).
//!
//! In biber these are *global* module state (`$ANN` in Annotation.pm),
//! keyed by citekey and shared by all sections; they are not part of a
//! section's state (and not in the phase dumps). bibers keeps them in
//! [`crate::pipeline::RunState::annotations`].

use std::collections::BTreeMap;

/// One annotation value.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Annotation {
    /// The annotation text (`undef` is possible, BB: research 01 B11).
    pub value: Option<String>,
    /// Whether the annotation was a quoted literal.
    pub literal: bool,
}

/// name → annotation.
pub type ByName<T> = BTreeMap<String, T>;
/// field → name → …
pub type ByField<T> = BTreeMap<String, ByName<T>>;

/// count → part → annotation.
pub type PartMap = BTreeMap<String, BTreeMap<String, Annotation>>;

/// biber's `$ANN`. Perl `undef` hash keys (a missing item count) are `""`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Annotations {
    /// `{field}{key}{field}{name}`.
    pub field: BTreeMap<String, ByField<Annotation>>,
    /// `{item}{key}{field}{name}{count}`.
    pub item: BTreeMap<String, ByField<BTreeMap<String, Annotation>>>,
    /// `{part}{key}{field}{name}{count}{part}`.
    pub part: BTreeMap<String, ByField<PartMap>>,
    /// `{fields}{key}{field}`: annotated fields.
    pub fields: BTreeMap<String, BTreeMap<String, bool>>,
    /// `{names}{key}{field}`: annotation names in first-seen order.
    pub names: BTreeMap<String, BTreeMap<String, Vec<String>>>,
}

/// The scope of an annotation (`set_annotation`'s first argument).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AnnotationScope {
    Field,
    Item { count: String },
    Part { count: String, part: String },
}

impl Annotations {
    /// `Biber::Annotation->set_annotation` (Annotation.pm:31).
    pub fn set_annotation(
        &mut self,
        scope: AnnotationScope,
        key: &str,
        field: &str,
        name: &str,
        value: Option<String>,
        literal: bool,
    ) {
        let ann = Annotation { value, literal };
        match scope {
            AnnotationScope::Field => {
                self.field
                    .entry(key.into())
                    .or_default()
                    .entry(field.into())
                    .or_default()
                    .insert(name.into(), ann);
            }
            AnnotationScope::Item { count } => {
                self.item
                    .entry(key.into())
                    .or_default()
                    .entry(field.into())
                    .or_default()
                    .entry(name.into())
                    .or_default()
                    .insert(count, ann);
            }
            AnnotationScope::Part { count, part } => {
                self.part
                    .entry(key.into())
                    .or_default()
                    .entry(field.into())
                    .or_default()
                    .entry(name.into())
                    .or_default()
                    .entry(count)
                    .or_default()
                    .insert(part, ann);
            }
        }
        self.fields
            .entry(key.into())
            .or_default()
            .insert(field.into(), true);
        let names = self
            .names
            .entry(key.into())
            .or_default()
            .entry(field.into())
            .or_default();
        let fname = perl_compat::fc(name);
        if !names.iter().any(|n| perl_compat::fc(n) == fname) {
            names.push(name.into());
        }
    }

    /// `is_annotated_field`.
    pub fn is_annotated_field(&self, key: &str, field: &str) -> bool {
        self.fields
            .get(key)
            .is_some_and(|m| m.get(field).copied().unwrap_or(false))
    }
}

fn ann_json(a: &Annotation) -> serde_json::Value {
    serde_json::json!({
        "value": a.value,
        "literal": if a.literal { "1" } else { "0" },
    })
}

fn ann_from_json(v: &serde_json::Value) -> Annotation {
    Annotation {
        value: v.get("value").and_then(|x| x.as_str()).map(str::to_string),
        literal: v
            .get("literal")
            .and_then(|x| x.as_str())
            .is_some_and(perl_compat::truth::perl_truthy),
    }
}

impl Annotations {
    /// The annotations of `keys` in the phase dumps' shape (`state.annotations`
    /// in `BiberPhaseDump.pm`): `{key: {field: {f: {name: {value, literal}}},
    /// item: {f: {name: {count: …}}}, part: {f: {name: {count: {part: …}}}}}}`,
    /// omitting keys without annotations.
    pub fn to_dump<'a>(&self, keys: impl IntoIterator<Item = &'a str>) -> serde_json::Value {
        use serde_json::{Map, Value};
        let mut out = Map::new();
        for key in keys {
            let mut k = Map::new();
            if let Some(fields) = self.field.get(key) {
                let m: Map<String, Value> = fields
                    .iter()
                    .map(|(f, names)| {
                        (
                            f.clone(),
                            Value::Object(
                                names
                                    .iter()
                                    .map(|(n, a)| (n.clone(), ann_json(a)))
                                    .collect(),
                            ),
                        )
                    })
                    .collect();
                if !m.is_empty() {
                    k.insert("field".into(), Value::Object(m));
                }
            }
            if let Some(fields) = self.item.get(key) {
                let m: Map<String, Value> = fields
                    .iter()
                    .map(|(f, names)| {
                        let names = names
                            .iter()
                            .map(|(n, counts)| {
                                let c = counts
                                    .iter()
                                    .map(|(c, a)| (c.clone(), ann_json(a)))
                                    .collect();
                                (n.clone(), Value::Object(c))
                            })
                            .collect();
                        (f.clone(), Value::Object(names))
                    })
                    .collect();
                if !m.is_empty() {
                    k.insert("item".into(), Value::Object(m));
                }
            }
            if let Some(fields) = self.part.get(key) {
                let m: Map<String, Value> = fields
                    .iter()
                    .map(|(f, names)| {
                        let names = names
                            .iter()
                            .map(|(n, counts)| {
                                let c = counts
                                    .iter()
                                    .map(|(c, parts)| {
                                        let p = parts
                                            .iter()
                                            .map(|(p, a)| (p.clone(), ann_json(a)))
                                            .collect();
                                        (c.clone(), Value::Object(p))
                                    })
                                    .collect();
                                (n.clone(), Value::Object(c))
                            })
                            .collect();
                        (f.clone(), Value::Object(names))
                    })
                    .collect();
                if !m.is_empty() {
                    k.insert("part".into(), Value::Object(m));
                }
            }
            if !k.is_empty() {
                out.insert(key.to_string(), Value::Object(k));
            }
        }
        Value::Object(out)
    }

    /// Merge annotations from the phase dumps' shape (see [`Self::to_dump`])
    /// into this store, replacing the listed keys' annotations. Annotation
    /// names are recorded in sorted order (the dump does not keep biber's
    /// first-seen order).
    pub fn install_dump(&mut self, dump: &serde_json::Value) {
        let Some(keys) = dump.as_object() else {
            return;
        };
        for (key, k) in keys {
            self.field.remove(key);
            self.item.remove(key);
            self.part.remove(key);
            self.fields.remove(key);
            self.names.remove(key);
            let obj = |v: &serde_json::Value| v.as_object().cloned().unwrap_or_default();
            for (f, names) in obj(k.get("field").unwrap_or(&serde_json::Value::Null)) {
                for (n, a) in obj(&names) {
                    let a = ann_from_json(&a);
                    self.set_annotation(AnnotationScope::Field, key, &f, &n, a.value, a.literal);
                }
            }
            for (f, names) in obj(k.get("item").unwrap_or(&serde_json::Value::Null)) {
                for (n, counts) in obj(&names) {
                    for (c, a) in obj(&counts) {
                        let a = ann_from_json(&a);
                        let scope = AnnotationScope::Item { count: c };
                        self.set_annotation(scope, key, &f, &n, a.value, a.literal);
                    }
                }
            }
            for (f, names) in obj(k.get("part").unwrap_or(&serde_json::Value::Null)) {
                for (n, counts) in obj(&names) {
                    for (c, parts) in obj(&counts) {
                        for (p, a) in obj(&parts) {
                            let a = ann_from_json(&a);
                            let scope = AnnotationScope::Part {
                                count: c.clone(),
                                part: p,
                            };
                            self.set_annotation(scope, key, &f, &n, a.value, a.literal);
                        }
                    }
                }
            }
        }
    }
}

impl Annotations {
    /// `Biber::Annotation->inherit_annotations` (Annotation.pm:65): copy
    /// the annotations of `sourcekey`'s `sourcefield` to `targetkey`'s
    /// `targetfield`. With `items = (sourceitem, targetitem)` only that item
    /// (list position) of item and part annotations is copied.
    pub fn inherit_annotations(
        &mut self,
        sourcekey: &str,
        targetkey: &str,
        sourcefield: &str,
        targetfield: &str,
        items: Option<(&str, &str)>,
    ) {
        match items {
            None => {
                if let Some(v) = self.field.get(sourcekey).and_then(|m| m.get(sourcefield)) {
                    let v = v.clone();
                    self.field
                        .entry(targetkey.into())
                        .or_default()
                        .insert(targetfield.into(), v);
                }
                if let Some(v) = self.item.get(sourcekey).and_then(|m| m.get(sourcefield)) {
                    let v = v.clone();
                    self.item
                        .entry(targetkey.into())
                        .or_default()
                        .insert(targetfield.into(), v);
                }
                if let Some(v) = self.part.get(sourcekey).and_then(|m| m.get(sourcefield)) {
                    let v = v.clone();
                    self.part
                        .entry(targetkey.into())
                        .or_default()
                        .insert(targetfield.into(), v);
                }
            }
            Some((si, ti)) => {
                // `keys %{…}`: hash order in biber; sorted here (only the
                // order of the `names` list depends on it).
                if let Some(names) = self
                    .item
                    .get(sourcekey)
                    .and_then(|m| m.get(sourcefield))
                    .cloned()
                {
                    for (name, counts) in names {
                        self.add_name(targetkey, targetfield, &name);
                        if let Some(a) = counts.get(si) {
                            self.item
                                .entry(targetkey.into())
                                .or_default()
                                .entry(targetfield.into())
                                .or_default()
                                .entry(name.clone())
                                .or_default()
                                .insert(ti.into(), a.clone());
                        }
                    }
                }
                if let Some(names) = self
                    .part
                    .get(sourcekey)
                    .and_then(|m| m.get(sourcefield))
                    .cloned()
                {
                    for (name, counts) in names {
                        self.add_name(targetkey, targetfield, &name);
                        if let Some(a) = counts.get(si) {
                            self.part
                                .entry(targetkey.into())
                                .or_default()
                                .entry(targetfield.into())
                                .or_default()
                                .entry(name.clone())
                                .or_default()
                                .insert(ti.into(), a.clone());
                        }
                    }
                }
            }
        }
        self.mark_fields(targetkey);
    }

    /// `Biber::Annotation->copy_annotations` (Annotation.pm:121): copy all
    /// annotations of one entry to another (related entry clones).
    pub fn copy_annotations(&mut self, sourcekey: &str, targetkey: &str) {
        if let Some(v) = self.field.get(sourcekey).cloned() {
            self.field.insert(targetkey.into(), v);
        }
        if let Some(v) = self.item.get(sourcekey).cloned() {
            self.item.insert(targetkey.into(), v);
        }
        if let Some(v) = self.part.get(sourcekey).cloned() {
            self.part.insert(targetkey.into(), v);
        }
        if let Some(v) = self.names.get(sourcekey).cloned() {
            self.names.insert(targetkey.into(), v);
        }
        self.mark_fields(targetkey);
    }

    fn add_name(&mut self, key: &str, field: &str, name: &str) {
        let names = self
            .names
            .entry(key.into())
            .or_default()
            .entry(field.into())
            .or_default();
        let fname = perl_compat::fc(name);
        if !names.iter().any(|n| perl_compat::fc(n) == fname) {
            names.push(name.into());
        }
    }

    /// `$ANN->{fields}{$key}{$f} = 1` for every annotated field of `key`.
    fn mark_fields(&mut self, key: &str) {
        let mut fs: Vec<String> = Vec::new();
        if let Some(m) = self.field.get(key) {
            fs.extend(m.keys().cloned());
        }
        if let Some(m) = self.item.get(key) {
            fs.extend(m.keys().cloned());
        }
        if let Some(m) = self.part.get(key) {
            fs.extend(m.keys().cloned());
        }
        if !fs.is_empty() {
            let e = self.fields.entry(key.into()).or_default();
            for f in fs {
                e.insert(f, true);
            }
        }
    }
}
