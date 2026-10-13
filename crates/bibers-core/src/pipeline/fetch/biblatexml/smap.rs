//! The biblatexml source map engine: the map half of
//! `Biber::Input::file::biblatexml::create_entry` (biblatexml.pm:296-816),
//! with `_changenode` (biblatexml.pm:1450) and `_getpath`
//! (biblatexml.pm:1546). Fields are addressed by XPath over the entry's XML
//! (a field name `f` stands for `./bltx:f`, a name list for
//! `./bltx:names[@type='f']`).
//!
//! Crash fixes (biber dies; see the module docs of the parent): `per_type`
//! (`$entry->type` does not exist: compared with the `entrytype` attribute,
//! as intended), `map_notfield` without `map_field_source` (an empty XPath:
//! the field part of the step is skipped, as in the bibtex engine),
//! `map_matches` (`$etarget->set` does not exist: the field is set), and
//! every XPath or `_changenode` error (an `ERROR`, exit 2).

use super::{SrcRef, die, findnodes, instantiate, xpath_die_msg};
use crate::bcf::sourcemap::{MapStep, SourceMap, SourceMaps};
use crate::debug;
use crate::log::Fatal;
use crate::model::SectionState;
use crate::perlish::{fc, lc, nfc};
use crate::pipeline::Ctx;
use crate::pipeline::fetch::{FetchOpts, SecExt};
use crate::pipeline::sourcemap::{appendstrict_check, maploopreplace, select_maps};
use crate::quirks::Quirk;
use crate::xmldom::xpath::XPath;
use crate::xmldom::{Dom, Kind, NodeId};
use perl_compat::regex::{CaptureDigits, expand_imatches, imatch, ireplace, split_default_xsvsep};
use perl_compat::truth::{num_cmp_str, perl_truthy, perl_truthy_opt};
use std::cmp::Ordering;

enum Flow {
    NextStep,
    NextMap,
}

enum StepResult {
    Continue,
    Flow(Flow),
    Null,
}

/// A value given to `_changenode`: a string, or (`$value =~ m|/|`) a copy
/// of the node the XPath selects.
enum CnVal {
    Str(String),
    Node(NodeId),
}

struct Engine<'a, 'b> {
    ctx: &'a mut Ctx<'b>,
    st: &'a mut SectionState,
    opts: &'a FetchOpts,
    src: SrcRef<'a>,
    dom: &'a mut Dom,
    key: String,
    entry: NodeId,
    /// `%newentries`, in creation order.
    news: Vec<(String, NodeId)>,
}

/// Apply the biblatexml source maps to `entry`. Returns `None` when an
/// `map_entry_null` step drops the entry, else the entries the maps created
/// (in creation order; biber iterates `%newentries` in hash order).
#[allow(clippy::too_many_arguments)]
pub fn apply(
    ctx: &mut Ctx,
    st: &mut SectionState,
    opts: &FetchOpts,
    src: SrcRef,
    dom: &mut Dom,
    key: &str,
    entry: NodeId,
    rkeys: &mut Vec<String>,
) -> Result<Option<Vec<NodeId>>, Fatal> {
    let smaps: Vec<SourceMaps> = select_maps(ctx.cfg, "biblatexml")
        .into_iter()
        .cloned()
        .collect();
    if smaps.is_empty() {
        return Ok(Some(Vec::new()));
    }
    let mut eng = Engine {
        ctx,
        st,
        opts,
        src,
        dom,
        key: key.to_string(),
        entry,
        news: Vec::new(),
    };
    for smap in &smaps {
        if !eng.smap(smap, rkeys)? {
            return Ok(None);
        }
    }
    Ok(Some(eng.news.into_iter().map(|(_, n)| n).collect()))
}

/// `_getpath` (biblatexml.pm:1546).
fn getpath(ctx: &Ctx, opts: &FetchOpts, s: Option<&str>) -> Option<String> {
    let s = s.filter(|s| perl_truthy(s))?;
    if s.contains('/') {
        return Some(s.to_string());
    }
    if ctx
        .dm
        .is_field(s, &opts.annotation_marker, &opts.named_annotation_marker)
    {
        if ctx.dm.get_fieldtype(s) == Some("list") && ctx.dm.get_datatype(s) == Some("name") {
            return Some(format!("./bltx:names[@type='{s}']"));
        }
        return Some(format!("./bltx:{s}"));
    }
    Some(s.to_string())
}

impl Engine<'_, '_> {
    fn overwrite(smap: &SourceMaps, map: &SourceMap) -> bool {
        let v = map
            .map_overwrite
            .as_deref()
            .or(smap.map_overwrite.as_deref())
            .unwrap_or("0");
        perl_truthy(v)
    }

    fn mlr(&mut self, step: &MapStep, attr: &str, ml: &str) -> Option<String> {
        let fixed = !self.ctx.cfg.quirks.on(Quirk::SmapMapuniqOutsideForeach);
        maploopreplace(&mut self.ctx.cfg.sourcemap_run, step.get(attr), ml, fixed)
    }

    fn fail(&mut self, step: &MapStep, prefix: &str, msg: &str) -> StepResult {
        if perl_truthy_opt(step.get("map_final")) {
            debug!(
                self.ctx.log,
                "{prefix}{msg} and step has 'final' set, skipping rest of map ..."
            );
            StepResult::Flow(Flow::NextMap)
        } else {
            debug!(self.ctx.log, "{prefix}{msg}, skipping step ...");
            StepResult::Flow(Flow::NextStep)
        }
    }

    fn compile(&mut self, level: &str, x: &str) -> Result<XPath, Fatal> {
        XPath::compile(x).map_err(|e| {
            die(
                self.ctx,
                &format!(
                    "Source mapping (type={level}, key={}): invalid XPath '{x}': {e}",
                    self.key
                ),
            )
        })
    }

    fn exists(&self, node: NodeId, xp: &XPath) -> bool {
        super::exists(self.dom, node, xp)
    }

    fn findnodes(&mut self, node: NodeId, xpath: &str) -> Result<Vec<NodeId>, Fatal> {
        findnodes(self.ctx, self.dom, node, xpath, self.src.filename)
    }

    /// `$node->findvalue($xpath)`: the concatenated string values.
    fn findvalue(&mut self, node: NodeId, xpath: &str) -> Result<String, Fatal> {
        let ns = self.dom.in_scope_ns(node);
        let r = XPath::compile(xpath).and_then(|x| x.eval(self.dom, node, &ns));
        match r {
            Ok(v) => Ok(crate::xmldom::xpath::literal_value(self.dom, &v)),
            Err(e) => Err(die(self.ctx, &xpath_die_msg(self.src.filename, xpath, &e))),
        }
    }

    /// One `<maps>` (`$smap`). `Ok(false)`: `map_entry_null`.
    fn smap(&mut self, smap: &SourceMaps, rkeys: &mut Vec<String>) -> Result<bool, Fatal> {
        let level = smap.level.clone().unwrap_or_default();
        let secnum = self.st.section.number.clone();
        for map in &smap.maps {
            if let Some(rs) = &map.refsection
                && num_cmp_str(&secnum, rs) != Some(Ordering::Equal)
            {
                continue;
            }
            let entry = self.entry;
            let mut last_type = self
                .dom
                .get_attribute(entry, "entrytype")
                .map(str::to_string);
            let mut last_field: Option<String> = None;
            let mut last_fieldval: Option<String> = None;
            let mut imatches: Vec<Option<String>> = Vec::new();
            // per_type: biber calls `$entry->type`, which XML::LibXML does
            // not have (dies); compared with the entry type, as intended.
            let etype = self
                .dom
                .get_attribute(entry, "entrytype")
                .unwrap_or("")
                .to_string();
            if !map.per_type.is_empty()
                && !map
                    .per_type
                    .iter()
                    .any(|t| lc(t.as_deref().unwrap_or("")) == etype)
            {
                continue;
            }
            if map
                .per_nottype
                .iter()
                .any(|t| lc(t.as_deref().unwrap_or("")) == etype)
            {
                continue;
            }
            if !map.per_datasource.is_empty()
                && !map
                    .per_datasource
                    .iter()
                    .any(|d| d.as_deref().unwrap_or("") == self.src.filename)
            {
                continue;
            }
            let mut maploop: Vec<String> = vec![String::new()];
            if let Some(foreach) = map.map_foreach.as_deref().filter(|f| perl_truthy(f)) {
                let xp = if foreach.contains('/') {
                    foreach.to_string()
                } else {
                    format!("./bltx:{foreach}")
                };
                let felist = self.findnodes(entry, &xp)?;
                if !felist.is_empty() {
                    let s: String = felist.iter().map(|n| self.dom.text_content(*n)).collect();
                    maploop = split_default_xsvsep(&s);
                }
            }
            'ml: for ml in &maploop {
                for step in &map.steps {
                    match self.step(
                        &level,
                        smap,
                        map,
                        step,
                        ml,
                        rkeys,
                        &mut last_type,
                        &mut last_field,
                        &mut last_fieldval,
                        &mut imatches,
                    )? {
                        StepResult::Continue | StepResult::Flow(Flow::NextStep) => {}
                        StepResult::Flow(Flow::NextMap) => continue 'ml,
                        StepResult::Null => return Ok(false),
                    }
                }
            }
        }
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    fn step(
        &mut self,
        level: &str,
        smap: &SourceMaps,
        map: &SourceMap,
        step: &MapStep,
        ml: &str,
        rkeys: &mut Vec<String>,
        last_type: &mut Option<String>,
        last_field: &mut Option<String>,
        last_fieldval: &mut Option<String>,
        imatches: &mut Vec<Option<String>>,
    ) -> Result<StepResult, Fatal> {
        let truthy = |a: &str| perl_truthy_opt(step.get(a));
        let key = self.key.clone();
        let keyp = format!("Source mapping (type={level}, key={key}): ");

        if truthy("map_entry_null") {
            debug!(self.ctx.log, "{keyp}Ignoring entry completely");
            return Ok(StepResult::Null);
        }

        // new entry
        if let Some(newkey) = self
            .mlr(step, "map_entry_new", ml)
            .filter(|s| perl_truthy(s))
        {
            let Some(newtype) = self
                .mlr(step, "map_entry_newtype", ml)
                .filter(|s| perl_truthy(s))
            else {
                self.ctx.log.biber_warn(format!(
                    "{keyp}Missing type for new entry '{newkey}', skipping step ..."
                ));
                return Ok(StepResult::Flow(Flow::NextStep));
            };
            debug!(self.ctx.log, "{keyp}Creating new entry with key '{newkey}'");
            let ne = self.dom.new_element("bltx:entry");
            self.dom.set_attribute(ne, "id", &nfc(&newkey));
            self.dom.set_attribute(ne, "entrytype", &nfc(&newtype));
            rkeys.retain(|k| *k != newkey);
            if self.st.section.allkeys {
                self.st.section.add_citekeys(std::slice::from_ref(&newkey));
            }
            match self.news.iter_mut().find(|(k, _)| *k == newkey) {
                Some(slot) => slot.1 = ne,
                None => self.news.push((newkey, ne)),
            }
        }

        // entry clone: biber re-runs create_entry without maps on the
        // current entry, but that creates (overwrites) the entry under its
        // *own* id, so the clone key ends up cited without an entry and
        // biber dies in get_dependents. bibers creates the clone, as
        // intended: a copy of the entry in its current (partly mapped) state
        // under the clone key.
        if let Some(prefix) = self
            .mlr(step, "map_entry_clone", ml)
            .filter(|s| perl_truthy(s))
        {
            debug!(self.ctx.log, "{keyp}cloning entry with prefix '{prefix}'");
            let ck = format!("{prefix}{key}");
            let clone = self.dom.clone_deep(self.entry);
            self.dom.set_attribute(clone, "id", &ck);
            instantiate(
                self.ctx, self.st, self.opts, self.src, self.dom, clone, self.entry,
            )?;
            rkeys.retain(|k| *k != ck);
            if self.st.section.allkeys {
                self.st.section.add_citekeys(std::slice::from_ref(&ck));
            }
        }

        // entry target
        let (etarget, etargetkey) = match self
            .mlr(step, "map_entrytarget", ml)
            .filter(|s| perl_truthy(s))
        {
            Some(tk) => match self.news.iter().find(|(k, _)| *k == tk) {
                Some((_, n)) => (*n, tk),
                None => {
                    self.ctx.log.biber_warn(format!(
                        "{keyp}Dynamically created entry target '{tk}' does not exist skipping step ..."
                    ));
                    return Ok(StepResult::Flow(Flow::NextStep));
                }
            },
            None => (self.entry, key.clone()),
        };
        let tp = format!("Source mapping (type={level}, key={etargetkey}): ");

        // entry type map
        if let Some(ts) = self
            .mlr(step, "map_type_source", ml)
            .filter(|s| perl_truthy(s))
        {
            let ts = lc(&ts);
            let cur = self
                .dom
                .get_attribute(etarget, "entrytype")
                .unwrap_or("")
                .to_string();
            if cur != ts {
                return Ok(self.fail(
                    step,
                    &tp,
                    &format!("Entry type is '{cur}' but map wants '{ts}'"),
                ));
            }
            *last_type = Some(cur.clone());
            let t = lc(&self.mlr(step, "map_type_target", ml).unwrap_or_default());
            debug!(self.ctx.log, "{tp}Changing entry type from '{cur}' to {t}");
            self.dom.set_attribute(etarget, "entrytype", &nfc(&t));
        }

        let mut fieldcontinue = false;
        let mut xp_fieldsource_s: Option<String> = None;

        // negated source field map
        let nf = self.mlr(step, "map_notfield", ml);
        if let Some(xs) = getpath(self.ctx, self.opts, nf.as_deref()) {
            let xp = self.compile(level, &xs)?;
            if self.exists(etarget, &xp) {
                return Ok(self.fail(step, &tp, &format!("Field xpath '{xs}' exists")));
            }
            fieldcontinue = true;
        }

        // cited/nocited conditions
        let is_cite = self.st.section.cite_citekeys.contains(&key);
        let is_nocite = self.st.section.nocite_citekeys.contains(&key);
        let allnocite = self.st.section.allkeys_nocite;
        if truthy("map_entrykey_citedornocited") && !(is_cite || is_nocite) {
            return Ok(self.fail(step, &keyp, "Key is neither \\cited nor \\nocited"));
        }
        if truthy("map_entrykey_cited") && !is_cite {
            return Ok(self.fail(step, &keyp, "Key is not explicitly \\cited"));
        }
        if truthy("map_entrykey_nocited") && (is_cite || (!is_nocite && !allnocite)) {
            return Ok(self.fail(step, &keyp, "Key is not \\nocited"));
        }
        if truthy("map_entrykey_allnocited") && !allnocite {
            return Ok(self.fail(step, &keyp, "Key is not \\nocite{*}'ed"));
        }
        // BB-031: without \nocite{*} this applies to every entry.
        let not_starnocited = if self.ctx.cfg.quirks.on(Quirk::SmapStarnocited) {
            allnocite && (is_cite || is_nocite)
        } else {
            !allnocite || is_cite || is_nocite
        };
        if truthy("map_entrykey_starnocited") && not_starnocited {
            return Ok(self.fail(
                step,
                &keyp,
                "Key is \\nocite{*}'ed but also either \\cite'd or explicitly \\nocited",
            ));
        }

        // field map
        let fs = self.mlr(step, "map_field_source", ml);
        if let Some(xs) = getpath(self.ctx, self.opts, fs.as_deref()) {
            let xp = self.compile(level, &xs)?;
            if !self.exists(etarget, &xp) {
                return Ok(self.fail(step, &tp, &format!("No field xpath '{xs}'")));
            }
            xp_fieldsource_s = Some(xs);
            fieldcontinue = true;
        }

        // `map_notfield` alone: biber dies on `findnodes(undef)`; the field
        // part of the step does not apply.
        if fieldcontinue && let Some(xfs) = xp_fieldsource_s.clone() {
            let Some(&srcnode) = self.findnodes(etarget, &xfs)?.first() else {
                return Err(die(
                    self.ctx,
                    &format!("{tp}Field xpath '{xfs}' selects nothing (XPath error)"),
                ));
            };
            *last_field = Some(self.dom.node_name(srcnode));
            let lfv = self.findvalue(etarget, &xfs)?;
            *last_fieldval = Some(lfv.clone());

            // precedence of biber's in-place step edits
            let mut negmatch = false;
            let mut match_attr = "map_match";
            if truthy("map_notmatch") {
                match_attr = "map_notmatch";
                negmatch = true;
            }
            let mut ci = false;
            if truthy("map_matchi") {
                match_attr = "map_matchi";
                ci = true;
            } else if truthy("map_notmatchi") {
                match_attr = "map_notmatchi";
                ci = true;
            }
            let fixed_ci = !self.ctx.cfg.quirks.on(Quirk::BltxmlSmapMatchi);
            if fixed_ci && match_attr == "map_notmatchi" {
                negmatch = true;
            }
            let mut matches_attr = "map_matches";
            let mut cis = false;
            if truthy("map_matchesi") {
                matches_attr = "map_matchesi";
                cis = true;
            }

            if let Some(ms) = step.get(matches_attr).filter(|s| perl_truthy(s)) {
                let ms = split_default_xsvsep(ms);
                let rs = split_default_xsvsep(step.get("map_replace").unwrap_or(""));
                if ms.len() != rs.len() {
                    debug!(
                        self.ctx.log,
                        "{tp}Different number of fixed matches vs replaces, skipping ..."
                    );
                    return Ok(StepResult::Flow(Flow::NextStep));
                }
                for (m, r) in ms.iter().zip(&rs) {
                    if (cis && fc(&lfv) == fc(m)) || lfv == *m {
                        // biber calls a non-existent `$etarget->set` (dies)
                        let _ = self.changenode(level, etarget, &xfs, r, true)?;
                    }
                }
            }

            if let Some(m) = self.mlr(step, match_attr, ml).filter(|s| perl_truthy(s)) {
                if step.get("map_replace").is_some() {
                    if lc(&xfs) == "./@id" {
                        debug!(
                            self.ctx.log,
                            "{tp}Field xpath '{xfs}' is entrykey- cannot remap the value of this field, skipping ..."
                        );
                        return Ok(StepResult::Flow(Flow::NextStep));
                    }
                    let r = self.mlr(step, "map_replace", ml).unwrap_or_default();
                    debug!(
                        self.ctx.log,
                        "{tp}Doing match/replace '{m}' -> '{r}' on field xpath '{xfs}'"
                    );
                    let rep = match ireplace(&lfv, &m, &r, ci) {
                        Ok(rep) => rep,
                        Err(e) => {
                            return Err(die(
                                self.ctx,
                                &format!("Source mapping (type={level}, key={etargetkey}): {e}"),
                            ));
                        }
                    };
                    if let Some(code) = &rep.code_warning {
                        // DC-023
                        self.ctx.log.biber_warn(format!(
                            "{tp}Replacement '{r}' contains Perl code ({code}), which is not executed: using the regular expression as the replacement"
                        ));
                    }
                    // misplaced parenthesis: `_changenode` gets no error
                    // reference, so its error paths die
                    let _ = self.changenode(level, etarget, &xfs, &rep.value, true)?;
                } else {
                    let m = expand_imatches(&m, imatches, CaptureDigits::Single);
                    // BB-new: `imatch` is called without the case-insensitive
                    // flag
                    let res = match imatch(&lfv, &m, negmatch, ci && fixed_ci) {
                        Ok(r) => r,
                        Err(e) => {
                            return Err(die(
                                self.ctx,
                                &format!("Source mapping (type={level}, key={etargetkey}): {e}"),
                            ));
                        }
                    };
                    *imatches = res;
                    if imatches.is_empty() {
                        return Ok(self.fail(
                            step,
                            &tp,
                            &format!("Field xpath '{xfs}' does not match '{m}'"),
                        ));
                    }
                }
            }

            // set to a different target
            let ft = self.mlr(step, "map_field_target", ml);
            if let Some(xt) = getpath(self.ctx, self.opts, ft.as_deref()) {
                let xpt = self.compile(level, &xt)?;
                if lc(&xt) == "./@id" {
                    debug!(
                        self.ctx.log,
                        "{tp}Field xpath '{xfs}' is entrykey - cannot map this to a new field as you must have an entrykey, skipping ..."
                    );
                    return Ok(StepResult::Flow(Flow::NextStep));
                }
                if self.exists(etarget, &xpt) {
                    if Self::overwrite(smap, map) {
                        debug!(self.ctx.log, "{tp}Overwriting existing field xpath '{xt}'");
                    } else {
                        debug!(
                            self.ctx.log,
                            "{tp}Field xpath '{xfs}' is mapped to field xpath '{xt}' but both are defined, skipping ..."
                        );
                        return Ok(StepResult::Flow(Flow::NextStep));
                    }
                }
                if let Err(e) = self.changenode(level, etarget, &xt, &xfs, false)? {
                    self.ctx.log.biber_warn(format!("{keyp}{e}"));
                }
                match self.findnodes(etarget, &xfs)?.first() {
                    Some(&n) => self.dom.unbind(n),
                    None => {
                        return Err(die(
                            self.ctx,
                            &format!("{tp}Field xpath '{xfs}' selects nothing (XPath error)"),
                        ));
                    }
                }
            }
        }

        // field changes
        let fset = self.mlr(step, "map_field_set", ml);
        if let Some(xn) = getpath(self.ctx, self.opts, fset.as_deref()) {
            let xp = self.compile(level, &xn)?;
            if truthy("map_null") {
                debug!(self.ctx.log, "{tp}Deleting field xpath '{xn}'");
                // biber dies (unbindNode on undef) when there is no such
                // field; deleting nothing is a no-op, as in the bibtex engine
                if let Some(&n) = self.findnodes(etarget, &xn)?.first() {
                    self.dom.unbind(n);
                }
            } else {
                if self.exists(etarget, &xp) && !Self::overwrite(smap, map) {
                    if truthy("map_final") {
                        debug!(
                            self.ctx.log,
                            "{tp}Field xpath '{xn}' exists, overwrite is not set and step has 'final' set, skipping rest of map ..."
                        );
                        return Ok(StepResult::Flow(Flow::NextMap));
                    }
                    debug!(
                        self.ctx.log,
                        "{tp}Field xpath '{xn}' exists and overwrite is not set, skipping step ..."
                    );
                    return Ok(StepResult::Flow(Flow::NextStep));
                }
                let mut orig = String::new();
                if truthy("map_append") || truthy("map_appendstrict") {
                    orig = self.findvalue(etarget, &xn)?;
                    if !perl_truthy(&orig) {
                        orig = String::new();
                    }
                }
                let (val, wkey) = if truthy("map_origentrytype") {
                    let Some(lt) = last_type.clone().filter(|t| perl_truthy(t)) else {
                        return Ok(StepResult::Flow(Flow::NextStep));
                    };
                    debug!(self.ctx.log, "{tp}Setting xpath '{xn}' to '{orig}{lt}'");
                    (appendstrict_check(step, &orig, &lt), &keyp)
                } else if truthy("map_origfieldval") {
                    let Some(lfv) = last_fieldval.clone().filter(|v| perl_truthy(v)) else {
                        return Ok(StepResult::Flow(Flow::NextStep));
                    };
                    debug!(
                        self.ctx.log,
                        "{tp}Setting field xpath '{xn}' to '{orig}{lfv}'"
                    );
                    (appendstrict_check(step, &orig, &lfv), &tp)
                } else if truthy("map_origfield") {
                    let Some(lf) = last_field.clone().filter(|v| perl_truthy(v)) else {
                        return Ok(StepResult::Flow(Flow::NextStep));
                    };
                    debug!(
                        self.ctx.log,
                        "{tp}Setting field xpath '{xn}' to '{orig}{lf}'"
                    );
                    (appendstrict_check(step, &orig, &lf), &tp)
                } else {
                    let fv = self.mlr(step, "map_field_value", ml).unwrap_or_default();
                    let fv = expand_imatches(&fv, imatches, CaptureDigits::Single);
                    debug!(
                        self.ctx.log,
                        "{tp}Setting field xpath '{xn}' to '{orig}{fv}'"
                    );
                    (appendstrict_check(step, &orig, &fv), &keyp)
                };
                let wkey = wkey.clone();
                if let Err(e) = self.changenode(level, etarget, &xn, &val, false)? {
                    self.ctx.log.biber_warn(format!("{wkey}{e}"));
                }
            }
        }
        Ok(StepResult::Continue)
    }

    /// The string a node value becomes when used as a string: the
    /// serialised XML (Perl stringifies XML::LibXML nodes, BB-new), or its
    /// text with the quirk fixed.
    fn node_string(&self, n: NodeId) -> String {
        if self.ctx.cfg.quirks.on(Quirk::BltxmlSmapNodeString) {
            self.dom.to_xml_string(n)
        } else {
            self.dom.text_content(n)
        }
    }

    /// XML::LibXML dies with "bad name" creating an element or attribute
    /// with an invalid name (e.g. a step `bltx:name[2]` that selects nothing).
    fn bad_name(&mut self, n: &str) -> Fatal {
        die(
            self.ctx,
            &format!(
                "Source mapping (key={}): cannot create a node named '{n}' (not a valid XML name)",
                self.key
            ),
        )
    }

    /// `_changenode` (biblatexml.pm:1450). `Ok(Err(msg))` is biber's
    /// `$$error`; with `no_error_ref` (callers that pass none) those paths
    /// die.
    fn changenode(
        &mut self,
        level: &str,
        e: NodeId,
        xp_target_s: &str,
        value: &str,
        no_error_ref: bool,
    ) -> Result<Result<(), String>, Fatal> {
        let r = self.changenode_inner(e, xp_target_s, value)?;
        if no_error_ref && let Err(msg) = r {
            return Err(die(
                self.ctx,
                &format!("Source mapping (type={level}, key={}): {msg}", self.key),
            ));
        }
        Ok(r)
    }

    fn changenode_inner(
        &mut self,
        e: NodeId,
        xp_target_s: &str,
        value: &str,
    ) -> Result<Result<(), String>, Fatal> {
        let mut value = value.to_string();
        let o = self.opts;
        if self
            .ctx
            .dm
            .is_field(&value, &o.annotation_marker, &o.named_annotation_marker)
            && self.ctx.dm.get_fieldtype(&value) == Some("list")
            && self.ctx.dm.get_datatype(&value) == Some("name")
        {
            value = format!("./bltx:names[@type='{value}']");
        }
        let val = if value.contains('/') {
            match self.findnodes(e, &value)?.first() {
                Some(&n) => CnVal::Node(self.dom.clone_deep(n)),
                None => {
                    return Err(die(
                        self.ctx,
                        &format!(
                            "Source mapping (key={}): value '{value}' is taken as an XPath and selects nothing",
                            self.key
                        ),
                    ));
                }
            }
        } else {
            CnVal::Str(value)
        };
        let as_str = |s: &Self, v: &CnVal| match v {
            CnVal::Str(x) => nfc(x),
            CnVal::Node(n) => nfc(&s.node_string(*n)),
        };

        if let Some(&n) = self.findnodes(e, xp_target_s)?.first() {
            match self.dom.node(n).kind {
                Kind::Attribute { .. } => {
                    if matches!(val, CnVal::Node(_)) {
                        return Ok(Err(format!(
                            "Tried to replace '{xp_target_s}' Attribute node with complex data"
                        )));
                    }
                    let s = as_str(self, &val);
                    self.dom.set_value(n, &s);
                }
                Kind::Element { .. } => match val {
                    CnVal::Node(v) => {
                        self.dom.remove_children(n);
                        for cn in self.dom.children(v).to_vec() {
                            self.dom.append_child(n, cn);
                        }
                    }
                    CnVal::Str(_) => {
                        let s = as_str(self, &val);
                        match self
                            .dom
                            .children(n)
                            .iter()
                            .copied()
                            .find(|c| self.dom.is_text(*c))
                        {
                            Some(t) => self.dom.set_value(t, &s),
                            None => {
                                return Err(die(
                                    self.ctx,
                                    &format!(
                                        "Source mapping (key={}): target '{xp_target_s}' has no text to replace",
                                        self.key
                                    ),
                                ));
                            }
                        }
                    }
                },
                Kind::Text(_) => {
                    if matches!(val, CnVal::Node(_)) {
                        return Ok(Err(format!(
                            "Tried to replace '{xp_target_s}' Text node with complex data"
                        )));
                    }
                    let s = as_str(self, &val);
                    self.dom.set_value(n, &s);
                }
                _ => {}
            }
            return Ok(Ok(()));
        }

        let path = xp_target_s.strip_prefix("./").unwrap_or(xp_target_s);
        let nodes: Vec<&str> = path.split('/').collect();
        let mut nodepath = ".".to_string();
        let mut nodeparent = ".".to_string();
        for (i, node) in nodes.iter().enumerate() {
            let terminal = i + 1 == nodes.len();
            nodepath.push('/');
            nodepath.push_str(node);
            if self.findnodes(e, &nodepath)?.is_empty() {
                let Some(&parent) = self.findnodes(e, &nodeparent)?.first() else {
                    return Err(die(
                        self.ctx,
                        &format!(
                            "Source mapping (key={}): cannot create '{xp_target_s}' (no parent '{nodeparent}')",
                            self.key
                        ),
                    ));
                };
                if let Some(np) = node.strip_prefix("bltx:") {
                    let mut f = np.to_string();
                    if let Some(t) = names_type(np) {
                        f = t;
                    }
                    if self.ctx.dm.get_fieldtype(&f) == Some("list")
                        && self.ctx.dm.get_datatype(&f) == Some("name")
                    {
                        let nn = self.dom.new_element("names");
                        self.dom.append_child(parent, nn);
                        self.dom.set_namespace(nn, super::BLTX_NS, super::BLTX);
                        self.dom.set_attribute(nn, "type", &f);
                        if terminal {
                            match &val {
                                CnVal::Node(v) => {
                                    for cn in self.dom.children(*v).to_vec() {
                                        self.dom.append_child(nn, cn);
                                    }
                                }
                                CnVal::Str(_) => {
                                    return Ok(Err(format!(
                                        "Tried to map to complex target '{xp_target_s}' with string value"
                                    )));
                                }
                            }
                        }
                    } else {
                        if !valid_node_name(np) {
                            return Err(self.bad_name(np));
                        }
                        let nn = self.dom.new_element(np);
                        self.dom.append_child(parent, nn);
                        self.dom.set_namespace(nn, super::BLTX_NS, super::BLTX);
                        if terminal {
                            let s = as_str(self, &val);
                            self.dom.append_text(nn, &s);
                        }
                    }
                } else if let Some(an) = node.strip_prefix('@') {
                    if terminal {
                        if !valid_node_name(an) {
                            return Err(self.bad_name(an));
                        }
                        let s = as_str(self, &val);
                        self.dom.set_attribute(parent, an, &s);
                    }
                } else if node.ends_with("text()") && terminal {
                    let s = as_str(self, &val);
                    self.dom.append_text(parent, &s);
                }
            }
            nodeparent.push('/');
            nodeparent.push_str(node);
        }
        Ok(Ok(()))
    }
}

/// `LibXML_test_node_name` (XML::LibXML croaks "bad name" for others).
fn valid_node_name(s: &str) -> bool {
    let mut cs = s.chars();
    let Some(first) = cs.next() else {
        return false;
    };
    let combining = |c: char| matches!(c, '\u{300}'..='\u{36f}' | '\u{483}'..='\u{486}' | '\u{591}'..='\u{5c4}' | '\u{64b}'..='\u{652}');
    (first.is_alphabetic() || first == '_' || first == ':')
        && cs.all(|c| {
            c.is_alphanumeric() || matches!(c, '_' | '-' | ':' | '.' | '\u{b7}') || combining(c)
        })
}

/// `m/names\[\@type\s*=\s*'(.+)'\]/`.
fn names_type(np: &str) -> Option<String> {
    let i = np.find("names[@type")?;
    let rest = np[i + "names[@type".len()..].trim_start();
    let rest = rest.strip_prefix('=')?.trim_start().strip_prefix('\'')?;
    let end = rest.rfind("']")?;
    (end > 0).then(|| rest[..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::names_type;

    #[test]
    fn names_type_re() {
        assert_eq!(names_type("names[@type='author']"), Some("author".into()));
        assert_eq!(names_type("names[@type = 'a']"), Some("a".into()));
        assert_eq!(names_type("title"), None);
    }
}
