//! An emulation of Perl's Getopt::Long 2.58 as bin/biber configures it
//! (`:config no_ignore_case`; defaults otherwise: `auto_abbrev`,
//! `getopt_compat` (`+` starts options too), `permute`, no bundling).
//!
//! Supported option specs: flags (``), counters (`+`), negatable (`!`),
//! mandatory/optional strings and integers (`=s`, `:s`, `=i`, `:i`), optional
//! integers with a default (`:80`), and array (`@`) and hash destinations.

use std::collections::BTreeMap;

/// The type of an option's value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ty {
    /// A flag.
    Flag,
    /// A counter (`+`).
    Counter,
    /// A negatable flag (`!`).
    Negatable,
    /// A string.
    Str,
    /// An integer.
    Int,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dest {
    Scalar,
    Array,
    Hash,
}

#[derive(Clone, Debug)]
struct Ctl {
    ty: Ty,
    cname: String,
    default: Option<String>,
    dest: Dest,
    mandatory: bool,
}

/// A parsed option value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Val {
    /// A scalar (Perl numbers are kept in their string form).
    Scalar(String),
    /// An array destination.
    Array(Vec<String>),
    /// A hash destination.
    Hash(BTreeMap<String, String>),
}

/// The option table.
pub struct Getopt {
    /// name → control (aliases share the control).
    names: BTreeMap<String, usize>,
    ctls: Vec<Ctl>,
    /// Names matched only exactly, never by abbreviation (bibers
    /// extensions, so that they cannot make biber's abbreviations ambiguous).
    exact_only: BTreeMap<String, usize>,
}

/// The result of parsing.
#[derive(Debug, Default)]
pub struct Parsed {
    /// Canonical name → value.
    pub opts: BTreeMap<String, Val>,
    /// Non-option arguments (in order), then the arguments after `--`.
    pub rest: Vec<String>,
    /// Warnings printed to STDERR.
    pub warnings: Vec<String>,
}

impl Parsed {
    /// Did parsing succeed (`GetOptions` returned true)?
    pub fn ok(&self) -> bool {
        self.warnings.is_empty()
    }
}

fn is_int(s: &str) -> bool {
    // PAT_INT: [-+]?_*[0-9][0-9_]*
    let t = s.strip_prefix(['-', '+']).unwrap_or(s);
    let t = t.trim_start_matches('_');
    let mut chars = t.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_digit() || c == '_')
}

fn int_value(s: &str) -> String {
    let t: String = s.chars().filter(|&c| c != '_').collect();
    match t.parse::<i64>() {
        Ok(n) => n.to_string(),
        Err(_) => t.trim_start_matches('+').to_string(),
    }
}

impl Getopt {
    /// Build from Getopt::Long spec strings, e.g. `"quiet|q+"`.
    /// `hash_dest` lists canonical names whose destination is a hash.
    pub fn new(specs: &[&str], hash_dest: &[&str]) -> Self {
        let mut g = Getopt {
            names: BTreeMap::new(),
            ctls: Vec::new(),
            exact_only: BTreeMap::new(),
        };
        for spec in specs {
            g.add(spec, hash_dest, false);
        }
        g
    }

    /// Add an option that only matches when spelt out in full.
    pub fn add_exact_only(&mut self, spec: &str) {
        self.add(spec, &[], true);
    }

    fn add(&mut self, spec: &str, hash_dest: &[&str], exact: bool) {
        let split = spec.find(['=', ':', '!', '+']).unwrap_or(spec.len());
        let (names, suffix) = spec.split_at(split);
        let names: Vec<&str> = names.split('|').collect();
        let cname = names[0].to_string();
        let mut dest = if hash_dest.contains(&cname.as_str()) {
            Dest::Hash
        } else {
            Dest::Scalar
        };
        let (ty, default, mandatory) = match suffix {
            "" => (Ty::Flag, None, false),
            "+" => (Ty::Counter, None, false),
            "!" => (Ty::Negatable, None, false),
            s => {
                let mand = s.starts_with('=');
                let body = &s[1..];
                let body = if let Some(b) = body.strip_suffix('@') {
                    dest = Dest::Array;
                    b
                } else if let Some(b) = body.strip_suffix('%') {
                    dest = Dest::Hash;
                    b
                } else {
                    body
                };
                match body {
                    "s" => (Ty::Str, None, mand),
                    "i" | "n" => (Ty::Int, None, mand),
                    d => (Ty::Int, Some(int_value(d)), false),
                }
            }
        };
        let idx = self.ctls.len();
        self.ctls.push(Ctl {
            ty,
            cname,
            default,
            dest,
            mandatory,
        });
        let table = if exact {
            &mut self.exact_only
        } else {
            &mut self.names
        };
        for n in names {
            if ty == Ty::Negatable {
                table.insert(format!("no{n}"), idx);
                table.insert(format!("no-{n}"), idx);
            }
            table.insert(n.to_string(), idx);
        }
    }

    /// Parse `argv` (`GetOptions`).
    pub fn parse(&self, argv: &[String]) -> Parsed {
        let mut out = Parsed::default();
        let mut args: std::collections::VecDeque<String> = argv.iter().cloned().collect();
        let mut ret = Vec::new();
        while let Some(arg) = args.pop_front() {
            if arg == "--" {
                break;
            }
            match self.find_option(&arg, &mut args, &mut out.warnings) {
                Found::NotOption => ret.push(arg),
                Found::Error => {}
                Found::Opt { ctl, value, key } => {
                    let c = &self.ctls[ctl];
                    let entry = out.opts.get_mut(&c.cname);
                    match (c.dest, c.ty) {
                        (_, Ty::Counter) => {
                            let n = match entry {
                                Some(Val::Scalar(s)) => s.parse::<i64>().unwrap_or(0),
                                _ => 0,
                            };
                            let v: i64 = value.parse().unwrap_or(1);
                            out.opts
                                .insert(c.cname.clone(), Val::Scalar((n + v).to_string()));
                        }
                        (Dest::Array, _) => match entry {
                            Some(Val::Array(a)) => a.push(value),
                            _ => {
                                out.opts.insert(c.cname.clone(), Val::Array(vec![value]));
                            }
                        },
                        (Dest::Hash, _) => {
                            let k = key.unwrap_or_default();
                            match entry {
                                Some(Val::Hash(h)) => {
                                    h.insert(k, value);
                                }
                                _ => {
                                    out.opts.insert(
                                        c.cname.clone(),
                                        Val::Hash([(k, value)].into_iter().collect()),
                                    );
                                }
                            }
                        }
                        (Dest::Scalar, _) => {
                            out.opts.insert(c.cname.clone(), Val::Scalar(value));
                        }
                    }
                }
            }
        }
        ret.extend(args);
        out.rest = ret;
        out
    }

    fn find_option(
        &self,
        arg: &str,
        args: &mut std::collections::VecDeque<String>,
        warnings: &mut Vec<String>,
    ) -> Found {
        let (starter, mut opt) = if let Some(r) = arg.strip_prefix("--") {
            ("--", r)
        } else if let Some(r) = arg.strip_prefix('-') {
            ("-", r)
        } else if let Some(r) = arg.strip_prefix('+') {
            ("+", r)
        } else {
            return Found::NotOption;
        };
        if arg == "-" {
            return Found::NotOption;
        }
        let mut optarg = None;
        if let Some(pos) = opt.get(1..).and_then(|s| s.find('=')).map(|p| p + 1) {
            optarg = Some(opt[pos + 1..].to_string());
            opt = &opt[..pos];
        }
        let mut tryopt = opt.to_string();
        let exact_ext = self.exact_only.get(opt).copied();
        if exact_ext.is_none() && !opt.is_empty() {
            let hits: Vec<&String> = self.names.keys().filter(|n| n.starts_with(opt)).collect();
            let exact = hits.iter().filter(|h| h.as_str() == opt).count() == 1;
            let mut hits_c: Vec<String> = hits.iter().map(|h| (*h).clone()).collect();
            if !(hits.len() <= 1 || exact) {
                let mut cnames: Vec<&str> = hits
                    .iter()
                    .map(|h| self.ctls[self.names[*h]].cname.as_str())
                    .collect();
                cnames.sort_unstable();
                cnames.dedup();
                if cnames.len() != 1 {
                    warnings.push(format!(
                        "Option {opt} is ambiguous ({})",
                        hits.iter()
                            .map(|h| h.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                    return Found::Error;
                }
                hits_c = vec![cnames[0].to_string()];
            }
            if hits_c.len() == 1 && hits_c[0] != opt {
                tryopt = hits_c[0].clone();
            }
        }
        let ctl_idx = match exact_ext.or_else(|| self.names.get(&tryopt).copied()) {
            Some(i) => i,
            None => {
                if opt.is_empty() {
                    warnings.push(format!("Missing option after {starter}"));
                } else {
                    warnings.push(format!("Unknown option: {opt}"));
                }
                return Found::Error;
            }
        };
        let ctl = &self.ctls[ctl_idx];
        let name = tryopt;
        match ctl.ty {
            Ty::Flag | Ty::Counter | Ty::Negatable => {
                if optarg.is_some() {
                    warnings.push(format!("Option {name} does not take an argument"));
                    return Found::Error;
                }
                let v = if ctl.ty == Ty::Negatable && name != ctl.cname && name.starts_with("no") {
                    "0"
                } else {
                    "1"
                };
                return Found::Opt {
                    ctl: ctl_idx,
                    value: v.to_string(),
                    key: None,
                };
            }
            _ => {}
        }
        let no_arg = match &optarg {
            Some(a) => a.is_empty(),
            None => args.is_empty(),
        };
        if no_arg {
            if ctl.mandatory || ctl.dest == Dest::Hash {
                warnings.push(format!("Option {name} requires an argument"));
                return Found::Error;
            }
            let v = ctl.default.clone().unwrap_or_else(|| {
                if ctl.ty == Ty::Str {
                    String::new()
                } else {
                    "0".into()
                }
            });
            return Found::Opt {
                ctl: ctl_idx,
                value: v,
                key: None,
            };
        }
        let from_optarg = optarg.is_some();
        let mut a = match optarg {
            Some(a) => a,
            None => args.pop_front().unwrap_or_default(),
        };
        let mut key = None;
        if ctl.dest == Dest::Hash {
            match a.split_once('=') {
                Some((k, v)) => {
                    key = Some(k.to_string());
                    a = v.to_string();
                }
                None => {
                    let d = ctl.default.clone().or(if ctl.mandatory {
                        None
                    } else if ctl.ty == Ty::Str {
                        Some(String::new())
                    } else {
                        Some("1".into())
                    });
                    match d {
                        Some(d) => {
                            key = Some(a.clone());
                            a = d;
                        }
                        None => {
                            warnings.push(format!("Option {name}, key \"{a}\", requires a value"));
                            return Found::Error;
                        }
                    }
                }
            }
        }
        match ctl.ty {
            Ty::Str => {
                if !(ctl.mandatory || ctl.dest == Dest::Hash || from_optarg || a == "-")
                    && (a == "--" || is_option_like(&a))
                {
                    args.push_front(a);
                    a = String::new();
                }
            }
            Ty::Int => {
                if is_int(&a) {
                    a = int_value(&a);
                } else if from_optarg || ctl.mandatory {
                    warnings.push(format!(
                        "Value \"{a}\" invalid for option {name} (integer number expected)"
                    ));
                    return Found::Error;
                } else {
                    args.push_front(a);
                    a = ctl.default.clone().unwrap_or_else(|| "0".into());
                }
            }
            _ => {}
        }
        Found::Opt {
            ctl: ctl_idx,
            value: a,
            key,
        }
    }
}

/// `$arg =~ /^$prefix.+/` with prefix `(--|-|\+)`: a `-`/`+` followed by a
/// non-newline character.
fn is_option_like(a: &str) -> bool {
    let mut c = a.chars();
    matches!(c.next(), Some('-' | '+')) && c.next().is_some_and(|x| x != '\n')
}

enum Found {
    NotOption,
    Error,
    Opt {
        ctl: usize,
        value: String,
        key: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn basics() {
        let g = Getopt::new(
            &[
                "quiet|q+",
                "wraplines|w:80",
                "output_file|output-file|O=s",
                "collate_options|c=s",
                "tool",
                "tool_align|tool-align",
                "neg!",
            ],
            &["collate_options"],
        );
        let p = g.parse(&v(&[
            "-q", "x", "--quiet", "-w", "-c", "a=b", "-O", "-", "--tool", "--noneg", "--", "-y",
        ]));
        assert!(p.ok(), "{:?}", p.warnings);
        assert_eq!(p.opts["quiet"], Val::Scalar("2".into()));
        assert_eq!(p.opts["wraplines"], Val::Scalar("80".into()));
        assert_eq!(p.opts["output_file"], Val::Scalar("-".into()));
        assert_eq!(p.opts["tool"], Val::Scalar("1".into()));
        assert_eq!(p.opts["neg"], Val::Scalar("0".into()));
        assert_eq!(p.rest, v(&["x", "-y"]));
        let p = g.parse(&v(&["--tool-", "--zz"]));
        assert_eq!(p.warnings, vec!["Unknown option: zz".to_string()]);
        assert!(p.opts.contains_key("tool_align"));
    }
}
