//! Source map declarations (`\DeclareSourcemap` and biber.conf
//! `<sourcemap>`): structure only. Execution belongs to the input stage.

use std::collections::BTreeMap;

use crate::xmlin::{XHash, XVal};

/// One `<maps>` element: a set of maps at one level for one datatype.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceMaps {
    /// `datatype` (bibtex, biblatexml).
    pub datatype: Option<String>,
    /// `level` (user, style, driver).
    pub level: Option<String>,
    /// `map_overwrite`.
    pub map_overwrite: Option<String>,
    /// The maps, in document order.
    pub maps: Vec<SourceMap>,
    /// Any other attributes/children, kept raw.
    pub extra: BTreeMap<String, XVal>,
}

/// One `<map>` element.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceMap {
    /// `map_overwrite`.
    pub map_overwrite: Option<String>,
    /// `map_foreach`.
    pub map_foreach: Option<String>,
    /// `refsection`.
    pub refsection: Option<String>,
    /// `<per_datasource>` contents.
    pub per_datasource: Vec<Option<String>>,
    /// `<per_type>` contents.
    pub per_type: Vec<Option<String>>,
    /// `<per_nottype>` contents.
    pub per_nottype: Vec<Option<String>>,
    /// The steps, in document order.
    pub steps: Vec<MapStep>,
    /// Any other attributes/children, kept raw.
    pub extra: BTreeMap<String, XVal>,
}

/// One `<map_step>`: its attributes (`map_field_source`, `map_match`, …).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MapStep {
    /// Attribute name → value.
    pub attrs: BTreeMap<String, String>,
    /// Anything that is not a plain string attribute, kept raw.
    pub extra: BTreeMap<String, XVal>,
}

impl MapStep {
    /// An attribute value.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.attrs.get(name).map(String::as_str)
    }
}

fn contents(h: &XHash, key: &str) -> Vec<Option<String>> {
    h.list(key)
        .iter()
        .map(|v| match v {
            XVal::Str(s) => Some(s.clone()),
            XVal::Hash(h) => h.content().map(str::to_string),
            XVal::Array(_) => None,
        })
        .collect()
}

impl SourceMaps {
    /// Build from an `XMLin` `<maps>` hash.
    pub fn from_xval(h: &XHash) -> Self {
        let mut extra = BTreeMap::new();
        let mut maps = Vec::new();
        for (k, v) in h.iter() {
            match k {
                "datatype" | "level" | "map_overwrite" => {}
                "map" => {
                    for m in h.list("map").hashes() {
                        maps.push(SourceMap::from_xval(m));
                    }
                }
                _ => {
                    extra.insert(k.to_string(), v.clone());
                }
            }
        }
        SourceMaps {
            datatype: h.str("datatype").map(str::to_string),
            level: h.str("level").map(str::to_string),
            map_overwrite: h.str("map_overwrite").map(str::to_string),
            maps,
            extra,
        }
    }

    /// The `XMLin` shape, as biber stores it (for differential dumps).
    pub fn to_json(&self) -> serde_json::Value {
        let mut m = serde_json::Map::new();
        for (k, v) in &self.extra {
            m.insert(k.clone(), v.to_json());
        }
        if let Some(v) = &self.datatype {
            m.insert("datatype".into(), v.clone().into());
        }
        if let Some(v) = &self.level {
            m.insert("level".into(), v.clone().into());
        }
        if let Some(v) = &self.map_overwrite {
            m.insert("map_overwrite".into(), v.clone().into());
        }
        if !self.maps.is_empty() {
            m.insert(
                "map".into(),
                serde_json::Value::Array(self.maps.iter().map(SourceMap::to_json).collect()),
            );
        }
        serde_json::Value::Object(m)
    }
}

impl SourceMap {
    /// Build from an `XMLin` `<map>` hash.
    pub fn from_xval(h: &XHash) -> Self {
        let mut extra = BTreeMap::new();
        let mut steps = Vec::new();
        for (k, v) in h.iter() {
            match k {
                "map_overwrite" | "map_foreach" | "refsection" | "per_datasource" | "per_type"
                | "per_nottype" => {}
                "map_step" => {
                    for s in h.list("map_step").hashes() {
                        let mut st = MapStep::default();
                        for (sk, sv) in s.iter() {
                            match sv {
                                XVal::Str(x) => {
                                    st.attrs.insert(sk.to_string(), x.clone());
                                }
                                other => {
                                    st.extra.insert(sk.to_string(), other.clone());
                                }
                            }
                        }
                        steps.push(st);
                    }
                }
                _ => {
                    extra.insert(k.to_string(), v.clone());
                }
            }
        }
        SourceMap {
            map_overwrite: h.str("map_overwrite").map(str::to_string),
            map_foreach: h.str("map_foreach").map(str::to_string),
            refsection: h.str("refsection").map(str::to_string),
            per_datasource: contents(h, "per_datasource"),
            per_type: contents(h, "per_type"),
            per_nottype: contents(h, "per_nottype"),
            steps,
            extra,
        }
    }

    /// The `XMLin` shape.
    pub fn to_json(&self) -> serde_json::Value {
        let mut m = serde_json::Map::new();
        for (k, v) in &self.extra {
            m.insert(k.clone(), v.to_json());
        }
        for (k, v) in [
            ("map_overwrite", &self.map_overwrite),
            ("map_foreach", &self.map_foreach),
            ("refsection", &self.refsection),
        ] {
            if let Some(v) = v {
                m.insert(k.into(), v.clone().into());
            }
        }
        for (k, v) in [
            ("per_datasource", &self.per_datasource),
            ("per_type", &self.per_type),
            ("per_nottype", &self.per_nottype),
        ] {
            if !v.is_empty() {
                m.insert(
                    k.into(),
                    serde_json::Value::Array(
                        v.iter()
                            .map(|c| {
                                let mut o = serde_json::Map::new();
                                if let Some(c) = c {
                                    o.insert("content".into(), c.clone().into());
                                }
                                serde_json::Value::Object(o)
                            })
                            .collect(),
                    ),
                );
            }
        }
        if !self.steps.is_empty() {
            m.insert(
                "map_step".into(),
                serde_json::Value::Array(
                    self.steps
                        .iter()
                        .map(|s| {
                            let mut o = serde_json::Map::new();
                            for (k, v) in &s.extra {
                                o.insert(k.clone(), v.to_json());
                            }
                            for (k, v) in &s.attrs {
                                o.insert(k.clone(), v.clone().into());
                            }
                            serde_json::Value::Object(o)
                        })
                        .collect(),
                ),
            );
        }
        serde_json::Value::Object(m)
    }
}
