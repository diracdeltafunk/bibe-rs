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

/// biber's `$ANN`. Perl `undef` hash keys (a missing item count) are `""`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Annotations {
    /// `{field}{key}{field}{name}`.
    pub field: BTreeMap<String, ByField<Annotation>>,
    /// `{item}{key}{field}{name}{count}`.
    pub item: BTreeMap<String, ByField<BTreeMap<String, Annotation>>>,
    /// `{part}{key}{field}{name}{count}{part}`.
    pub part: BTreeMap<String, ByField<BTreeMap<String, BTreeMap<String, Annotation>>>>,
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
