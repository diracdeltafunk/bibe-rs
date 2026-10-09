//! A grammar-based random `.bib` generator.
//!
//! Mostly realistic data (so uniqueness, labels, sorting and inheritance get
//! exercised with names, years and titles that collide across entries), with
//! adversarial values mixed in at low weight: odd name shapes, ISO 8601-2/EDTF
//! dates, abbreviated and roman ranges, LaTeX and non-ASCII text, entry graphs
//! (crossref/xref/xdata/sets/related/ids), `@string` macros and syntax errors.

use super::bib::{Cite, Entry, Field, FuzzInput, Item, Val};
use super::rng::Rng;

/// Generate input `index` of the run with `seed`.
pub fn generate(seed: u64, index: u64) -> FuzzInput {
    let mut g = Gen::new(Rng::for_case(seed, index));
    g.input()
}

struct Person {
    given: String,
    family: String,
    prefix: Option<String>,
    suffix: Option<String>,
}

struct Gen {
    r: Rng,
    persons: Vec<Person>,
    years: Vec<i64>,
    titles: Vec<String>,
    macros: Vec<String>,
    keys: Vec<String>,
    types: Vec<String>,
    xdata_keys: Vec<String>,
    /// Number of names in each XDATA entry's `author` (0: none).
    xdata_authors: Vec<usize>,
    aliases: Vec<String>,
}

const GIVEN: &[&str] = &[
    "John",
    "Jane",
    "Maria",
    "Jean-Paul",
    "José",
    "Émile",
    "Łukasz",
    "Søren",
    "Zoë",
    "Ægidius",
    "Ivan",
    "Olga",
    "Ming",
    "Hiroshi",
    "Ali",
    "Ørjan",
    "Ünal",
    "Hans Peter",
    "Anne-Marie",
    "Ch.",
    "J.",
    "J. R. R.",
    "J.R.R.",
    "D. E.",
    "D.~E.",
    "{\\'E}mile",
    "{\\v{S}}imon",
    "\\\"{O}mer",
    "Ana María",
    "Mary Ann",
    "{Th}omas",
    "{\\relax Ch}ristopher",
    "{\\c{C}}a{\\u{g}}lar",
    "Ludwig",
    "Alexander",
    "Alex",
    "A.",
    "Paul",
    "Pierre",
    "Ǆemal",
    "Ìsọ̀lá",
    "Ыкс",
    "Ιωάννης",
    "{John Paul}",
    "Jo{\\\"e}l",
];

const FAMILY: &[&str] = &[
    "Smith",
    "Knuth",
    "Müller",
    "Muller",
    "Mueller",
    "Ødegaard",
    "García Márquez",
    "{Garc\\'ia M\\'arquez}",
    "O'Brien",
    "McDonald",
    "d'Alembert",
    "D'Alembert",
    "{\\\"U}nal",
    "Ünal",
    "Schröder",
    "Schr{\\\"o}der",
    "Smythe-Jones",
    "Αριστοτέλης",
    "Толстой",
    "山田",
    "김",
    "Doe",
    "Zz",
    "Ábel",
    "Æsop",
    "Ölund",
    "{\\relax Ma}cdonald",
    "Brown",
    "Browne",
    "Lee",
    "Li",
    "Nguyễn",
    "Çelik",
    "Şahin",
    "Ångström",
    "Østergaard",
    "Jones",
    "Jönsson",
    "{\\AA}berg",
    "\\O{}stergaard",
    "Straße",
    "Strasse",
    "ßtart",
    "ǅukić",
    "Ńgo",
];

const PREFIX: &[&str] = &[
    "von",
    "van",
    "van der",
    "de",
    "de la",
    "da",
    "di",
    "du",
    "von und zu",
    "le",
    "della",
    "al",
    "ter",
    "{v}an",
    "{\\relax d}e",
    "dé",
    "ван",
];

const SUFFIX: &[&str] = &["Jr.", "Jr", "Sr.", "III", "II", "jr", "Junior"];

const CORPORATE: &[&str] = &[
    "{World Health Organization}",
    "{IEEE}",
    "{Smith and Sons}",
    "{{IEEE} Computer Society}",
    "{Ministère de l'Éducation}",
    "{The {R} Core Team}",
    "{\\relax Deutsche Bahn}",
    "{UNESCO, Paris}",
    "{Международный союз}",
];

const WORDS: &[&str] = &[
    "the",
    "art",
    "of",
    "computer",
    "programming",
    "theory",
    "on",
    "a",
    "new",
    "method",
    "for",
    "analysis",
    "and",
    "in",
    "studies",
    "history",
    "über",
    "die",
    "Geschichte",
    "la",
    "théorie",
    "des",
    "élites",
    "économie",
    "наука",
    "история",
    "日本語",
    "研究",
    "λόγος",
    "{DNA}",
    "{\\TeX}",
    "{\\LaTeX}",
    "Œuvres",
    "complètes",
    "naïve",
    "café",
    "Æthelred",
    "ﬁnal",
    "straße",
    "İstanbul",
    "ıi",
    "x",
    "Z",
    "1984",
    "2nd",
    "XIV",
];

const ARABIC_INDIC: [char; 10] = ['٠', '١', '٢', '٣', '٤', '٥', '٦', '٧', '٨', '٩'];
const DEVANAGARI: [char; 10] = ['०', '१', '२', '३', '४', '५', '६', '७', '८', '९'];

const ENTRY_TYPES: &[(u32, &str)] = &[
    (20, "article"),
    (16, "book"),
    (6, "inbook"),
    (8, "incollection"),
    (8, "inproceedings"),
    (5, "collection"),
    (5, "proceedings"),
    (4, "thesis"),
    (3, "phdthesis"),
    (2, "mastersthesis"),
    (4, "report"),
    (2, "techreport"),
    (5, "online"),
    (6, "misc"),
    (3, "manual"),
    (3, "mvbook"),
    (2, "periodical"),
    (2, "unpublished"),
    (2, "patent"),
    (2, "booklet"),
    (2, "suppbook"),
    (2, "bookinbook"),
    (1, "reference"),
    (1, "inreference"),
    (1, "mvcollection"),
    (1, "conference"),
    (1, "electronic"),
    (1, "www"),
    (1, "software"),
    (1, "dataset"),
    (1, "customa"),
    (1, "foo"),
];

const CONTAINED: &[&str] = &[
    "inbook",
    "incollection",
    "inproceedings",
    "bookinbook",
    "suppbook",
    "inreference",
    "conference",
];
const CONTAINERS: &[&str] = &[
    "book",
    "collection",
    "proceedings",
    "mvbook",
    "mvcollection",
    "reference",
    "periodical",
];

const NAME_FIELDS: &[(u32, &str)] = &[
    (40, "author"),
    (15, "editor"),
    (5, "translator"),
    (3, "bookauthor"),
    (2, "editora"),
    (2, "namea"),
    (2, "commentator"),
    (2, "shortauthor"),
    (1, "sortname"),
    (1, "foreword"),
    (1, "afterword"),
    (1, "holder"),
    (1, "shorteditor"),
];

const OPTIONS: &[&str] = &[
    "skipbib",
    "skipbib=true",
    "skipbib=false",
    "skipbiblist",
    "skiplab",
    "dataonly",
    "useprefix",
    "useprefix=true",
    "useprefix=false",
    "useauthor=false",
    "useeditor=false",
    "usetranslator=true",
    "uniquename=false",
    "uniquename=init",
    "uniquelist=false",
    "uniquelist=minyear",
    "labelalpha=true",
    "maxnames=1",
    "maxnames=2",
    "minnames=1",
    "maxalphanames=1",
    "maxbibnames=1",
    "maxcitenames=1",
    "singletitle=true",
    "uniquetitle=true",
    "uniquework=true",
    "labeldateparts=false",
    "labeltitle=true",
    "giveninits=true",
    "inheritance=false",
    "noinherit",
    "foo=bar",
    "=",
    "skipbib=0",
    "useprefix=1",
    "labelnamefield=editor",
    "sortingnamekeytemplatename=global",
];

impl Gen {
    fn new(r: Rng) -> Gen {
        Gen {
            r,
            persons: Vec::new(),
            years: Vec::new(),
            titles: Vec::new(),
            macros: Vec::new(),
            keys: Vec::new(),
            types: Vec::new(),
            xdata_keys: Vec::new(),
            xdata_authors: Vec::new(),
            aliases: Vec::new(),
        }
    }

    fn input(&mut self) -> FuzzInput {
        // Pools shared across the entries of this input, so values collide.
        let np = self.r.range(2, 7) as usize;
        for _ in 0..np {
            let p = Person {
                given: self.r.pick(GIVEN).to_string(),
                family: self.r.pick(FAMILY).to_string(),
                prefix: self.r.chance(0.2).then(|| self.r.pick(PREFIX).to_string()),
                suffix: self.r.chance(0.08).then(|| self.r.pick(SUFFIX).to_string()),
            };
            self.persons.push(p);
        }
        // Same family, different given name: uniquename pressure.
        if self.r.chance(0.5) {
            let fam = self.persons[0].family.clone();
            self.persons.push(Person {
                given: self.r.pick(GIVEN).to_string(),
                family: fam,
                prefix: None,
                suffix: None,
            });
        }
        let ny = self.r.range(1, 3) as usize;
        for _ in 0..ny {
            let y = self.r.range(1950, 2025);
            self.years.push(y);
        }
        for _ in 0..self.r.range(1, 4) {
            let t = self.title_text();
            self.titles.push(t);
        }

        let mut items = Vec::new();

        // @string macros (usually defined before use).
        let nmacros = *self.r.weighted(&[(6, 0usize), (3, 1), (2, 2), (1, 4)]);
        let mut late_macros = Vec::new();
        for i in 0..nmacros {
            let name = self
                .r
                .pick(&["jfoo", "pub", "acm", "ieee", "Loc", "x", "j_1", "LNCS"])
                .to_string();
            let name = format!("{name}{}", if self.r.chance(0.3) { "2" } else { "" });
            let value = if self.r.chance(0.2) && !self.macros.is_empty() {
                Val::Concat(vec![
                    Val::Bare(self.macros[0].clone()),
                    Val::Quoted(format!(" {}", self.word())),
                ])
            } else {
                let t = self.title_text();
                self.text_val(&t)
            };
            let item = Item::Macro {
                name: name.clone(),
                value,
            };
            self.macros.push(name);
            if i > 0 && self.r.chance(0.1) {
                late_macros.push(item);
            } else {
                items.push(item);
            }
        }
        if self.r.chance(0.12) {
            let v = self.r.pick(&[
                "\\newcommand{\\foo}{bar}",
                "\\providecommand{\\noopsort}[1]{}",
                "\\def\\x{y} % comment",
                "",
                "{\\relax}",
                "über \\'e",
            ]);
            items.push(Item::Preamble(Val::Braced(v.to_string())));
            if self.r.chance(0.3) {
                items.push(Item::Preamble(Val::Quoted("\\def\\y{z}".into())));
            }
        }
        if self.r.chance(0.08) {
            let c = self.r.pick(&[
                "jabref-meta: databaseType:biblatex;",
                "a comment with @ sign",
                "{nested {braces}}",
                "",
            ]);
            items.push(Item::Comment(c.to_string()));
        }

        // Entries: decide keys and types first so fields can reference them.
        let n = match self.r.below(10) {
            0..=2 => self.r.range(1, 3),
            3..=7 => self.r.range(4, 8),
            _ => self.r.range(9, 18),
        } as usize;
        for i in 0..n {
            let key = self.key(i);
            self.keys.push(key);
            let ty = self.r.weighted(ENTRY_TYPES).to_string();
            self.types.push(ty);
        }
        let nxdata = *self.r.weighted(&[(14, 0usize), (4, 1), (2, 2)]);
        for i in 0..nxdata {
            self.xdata_keys.push(format!("xd{}", i + 1));
        }

        // XDATA entries first, so granular references can target fields
        // that exist (an unresolved granular name reference makes biber's
        // uniqueness loop run forever).
        let mut xentries: Vec<Entry> = Vec::new();
        for i in 0..nxdata {
            let e = self.xdata_entry(i);
            xentries.push(e);
        }
        let mut entries: Vec<Entry> = Vec::new();
        for i in 0..n {
            let e = self.entry(i);
            entries.push(e);
        }
        entries.extend(xentries);
        // A @set entry.
        if n >= 2 && self.r.chance(0.12) {
            let mut members: Vec<String> = Vec::new();
            for _ in 0..self.r.range(1, 3) {
                let k = self.r.pick(&self.keys).clone();
                if !members.contains(&k) {
                    members.push(k);
                }
            }
            if self.r.chance(0.05) {
                members.push("nosuchmember".into());
            }
            let mut fields = vec![Field {
                name: "entryset".into(),
                value: Val::Braced(members.join(if self.r.chance(0.8) { "," } else { ", " })),
            }];
            if self.r.chance(0.3) {
                fields.push(Field {
                    name: "annotation".into(),
                    value: Val::Braced("set".into()),
                });
            }
            let key = format!("set{}", self.r.range(1, 9));
            self.keys.push(key.clone());
            entries.push(Entry {
                ty: if self.r.chance(0.9) { "set" } else { "Set" }.into(),
                key,
                fields,
                parens: false,
            });
        }
        // Order: mostly as generated, sometimes shuffled (parents before or
        // after children).
        if self.r.chance(0.3) {
            self.r.shuffle(&mut entries);
        }
        // Duplicate an entry key now and then.
        if !entries.is_empty() && self.r.chance(0.03) {
            let mut dup = entries[self.r.below(entries.len())].clone();
            if self.r.chance(0.5) {
                dup.key = dup.key.to_uppercase();
            }
            entries.push(dup);
        }
        for e in entries {
            items.push(Item::Entry(e));
        }
        items.extend(late_macros);

        // Syntax errors and junk (low weight).
        if self.r.chance(0.04) {
            let junk = self.junk();
            let pos = self.r.below(items.len() + 1);
            items.insert(pos, Item::Raw(junk));
        }
        if self.r.chance(0.04) {
            let pos = self.r.below(items.len() + 1);
            items.insert(
                pos,
                Item::Raw(
                    self.r
                        .pick(&["% a comment line", "junk text", "  "])
                        .to_string(),
                ),
            );
        }

        let (cites, nocite_all) = self.cites();
        let opts = self.option_tweaks();
        let args = self.cli_args();
        FuzzInput {
            items,
            cites,
            nocite_all,
            opts,
            args,
        }
    }

    /// Extra command-line options (none 70% of the time).
    fn cli_args(&mut self) -> Vec<String> {
        const ARGS: &[&str] = &[
            "--isbn-normalise",
            "--isbn13",
            "--decodecharsset=full",
            "--output-safechars",
            "--output-encoding=latin1",
            "--output-encoding=ascii",
            "--fixinits",
            "--mincrossrefs=1",
            "--minxrefs=1",
            "--sortcase=0",
            "--sortupper=0",
            "--sortlocale=de_DE",
            "--sortlocale=sv_SE",
            "--noxname",
            "--validate-datamodel",
            "--nostdmacros",
            "--xsvsep=\\s*;\\s*",
            "--listsep=&",
            "--namesep=und",
            "--others-string=etal",
            "--input-encoding=latin1",
            "--output-encoding=cp1252",
            "--output-encoding=latin2",
            "--output-safecharsset=full",
            "--decodecharsset=null",
            "--isbn-normalise",
            "--sortcase=false",
        ];
        let mut out: Vec<String> = Vec::new();
        if self.r.chance(0.6) {
            return out;
        }
        for _ in 0..self.r.range(1, 2) {
            let a = self.r.pick(ARGS).to_string();
            if !out.contains(&a) {
                out.push(a);
            }
        }
        out
    }

    /// Global option overrides (none half of the time).
    fn option_tweaks(&mut self) -> Vec<(String, String)> {
        const TWEAKS: &[(&str, &[&str])] = &[
            ("mincrossrefs", &["1", "3"]),
            ("minxrefs", &["1"]),
            ("labelalpha", &["1"]),
            ("labeltitle", &["1"]),
            ("labeltitleyear", &["1"]),
            ("labeldateparts", &["0", "1"]),
            ("singletitle", &["1"]),
            ("uniquetitle", &["1"]),
            ("uniquebaretitle", &["1"]),
            ("uniquework", &["1"]),
            ("uniqueprimaryauthor", &["1"]),
            ("maxcitenames", &["1", "2", "99"]),
            ("mincitenames", &["1", "2"]),
            ("maxbibnames", &["1", "99"]),
            ("maxsortnames", &["1", "2"]),
            ("minsortnames", &["1"]),
            ("maxalphanames", &["1", "2", "4"]),
            ("minalphanames", &["1", "2"]),
            (
                "uniquename",
                &[
                    "false", "init", "full", "allinit", "allfull", "mininit", "minfull",
                ],
            ),
            ("uniquelist", &["false", "true", "minyear"]),
            ("useprefix", &["0", "1"]),
            ("useauthor", &["0"]),
            ("useeditor", &["0"]),
            ("usetranslator", &["1"]),
            ("sortcase", &["0"]),
            ("sortupper", &["0"]),
            ("julian", &["1"]),
            ("nosortothers", &["1"]),
            ("nohashothers", &["1"]),
            ("noroman", &["1"]),
            ("maxitems", &["1", "2"]),
            (
                "sortlocale",
                &["auto", "ngerman", "swedish", "french", "spanish"],
            ),
        ];
        let mut out: Vec<(String, String)> = Vec::new();
        if self.r.chance(0.5) {
            return out;
        }
        for _ in 0..self.r.range(1, 3) {
            let (k, vs) = *self.r.pick(TWEAKS);
            if out.iter().any(|(x, _)| x == k) {
                continue;
            }
            out.push((k.to_string(), self.r.pick(vs).to_string()));
        }
        out
    }

    fn key(&mut self, i: usize) -> String {
        let base = match self.r.below(20) {
            0..=9 => format!("k{}", i + 1),
            10..=14 => {
                let fam = ascii_only(&self.persons[self.r.below(self.persons.len())].family);
                format!(
                    "{}{}{}",
                    if fam.is_empty() { "anon" } else { &fam },
                    self.years[0],
                    (b'a' + i as u8 % 26) as char
                )
            }
            15 => format!("smith:{}:{}", self.years[0], i),
            16 => format!("müller{}", i),
            17 => format!("K{}", i + 1),
            18 => format!("a.b_{}", i),
            _ => format!("{}", 100 + i),
        };
        if self.keys.contains(&base) {
            format!("{base}x{i}")
        } else {
            base
        }
    }

    fn entry(&mut self, i: usize) -> Entry {
        let ty = self.types[i].clone();
        let key = self.keys[i].clone();
        let mut fields: Vec<Field> = Vec::new();
        let mut add = |name: &str, value: Val| {
            fields.push(Field {
                name: name.into(),
                value,
            })
        };

        // Names.
        if self.r.chance(0.85) {
            let f = self.r.weighted(NAME_FIELDS).to_string();
            let v = self.name_list();
            add(&f, self.text_val(&v));
        }
        if self.r.chance(0.2) {
            let v = self.name_list();
            add("editor", self.text_val(&v));
        }
        if self.r.chance(0.04) {
            let v = self.name_list();
            add("translator", self.text_val(&v));
        }
        // Titles.
        if self.r.chance(0.9) {
            let t = if self.r.chance(0.3) {
                self.r.pick(&self.titles).clone()
            } else {
                self.title_text()
            };
            let v = self.maybe_macro_val(&t);
            add("title", v);
        }
        if self.r.chance(0.1) {
            let t = self.title_text();
            add("subtitle", self.text_val(&t));
        }
        if CONTAINED.contains(&ty.as_str()) && self.r.chance(0.5) {
            let t = self.title_text();
            add("booktitle", self.text_val(&t));
        }
        if self.r.chance(0.08) {
            let t = self.title_text();
            add("maintitle", self.text_val(&t));
        }
        if ty == "article" || self.r.chance(0.05) {
            let name = *self
                .r
                .pick(&["journaltitle", "journal", "journaltitle", "JOURNAL"]);
            let t = self.title_text();
            let v = self.maybe_macro_val(&t);
            add(name, v);
        }
        // Dates.
        match self.r.below(20) {
            0..=9 => {
                let d = self.date();
                add("date", self.text_val(&d));
            }
            10..=15 => {
                let y = self.year_value();
                add("year", y);
                if self.r.chance(0.4) {
                    let m = self.month_value();
                    add("month", m);
                }
                if self.r.chance(0.05) {
                    add("day", Val::Braced(self.r.range(0, 32).to_string()));
                }
            }
            16 => {
                let d = self.date();
                add("date", self.text_val(&d));
                let y = self.year_value();
                add("year", y);
                if self.r.chance(0.5) {
                    let m = self.month_value();
                    add("month", m);
                }
            }
            _ => {}
        }
        for df in ["origdate", "eventdate", "urldate"] {
            if self.r.chance(0.06) {
                let d = self.date();
                add(df, self.text_val(&d));
            }
        }
        if self.r.chance(0.02) {
            add(
                "origyear",
                Val::Braced(self.r.range(1500, 1990).to_string()),
            );
        }
        // Ranges and numbers.
        if self.r.chance(0.35) {
            let p = self.range_value();
            let name = if self.r.chance(0.9) { "pages" } else { "PAGES" };
            add(name, self.text_val(&p));
        }
        if self.r.chance(0.25) {
            let v = self.integerish();
            add("volume", v);
        }
        if self.r.chance(0.2) {
            let v = self.integerish();
            add("number", v);
        }
        if self.r.chance(0.08) {
            let v = self.integerish();
            add("volumes", v);
        }
        if self.r.chance(0.04) {
            let v = self.range_value();
            add("pagetotal", self.text_val(&v));
        }
        if self.r.chance(0.08) {
            let e = self
                .r
                .pick(&["2", "2nd", "Second", "{2}", "3rd ed.", "0", "Zweite"])
                .to_string();
            add("edition", Val::Braced(e));
        }
        // Lists.
        if self.r.chance(0.25) {
            let v = self.literal_list(&[
                "Springer",
                "{Addison-Wesley}",
                "Éditions du Seuil",
                "{Smith and Sons}",
                "MIT Press",
                "Наука",
                "O'Reilly",
                "\\relax Wiley",
            ]);
            let name = *self
                .r
                .pick(&["publisher", "publisher", "organization", "institution"]);
            add(name, self.text_val(&v));
        }
        if self.r.chance(0.2) {
            let v = self.literal_list(&[
                "Berlin",
                "New York",
                "München",
                "Paris and London",
                "{London and New York}",
                "Zürich",
                "Москва",
                "Reading, Mass.",
            ]);
            let name = *self.r.pick(&["location", "address", "location"]);
            add(name, self.text_val(&v));
        }
        if self.r.chance(0.06) {
            let v = self.literal_list(&[
                "english",
                "german",
                "langgerman",
                "french",
                "Latin",
                "langfrench",
            ]);
            add("language", Val::Braced(v));
        }
        if ty.contains("thesis") && self.r.chance(0.5) {
            add("school", Val::Braced("Universität Wien".into()));
        }
        if self.r.chance(0.05) {
            let t = self
                .r
                .pick(&["phdthesis", "mathesis", "Habil.", "candthesis"]);
            add("type", Val::Braced(t.to_string()));
        }
        if self.r.chance(0.12) {
            let t = self.title_text();
            add("series", self.maybe_macro_val(&t));
        }
        // Keywords and options.
        if self.r.chance(0.12) {
            let n = self.r.range(1, 4);
            let mut kws = Vec::new();
            for _ in 0..n {
                kws.push(
                    self.r
                        .pick(&[
                            "primary",
                            "secondary",
                            "a",
                            "B",
                            "key word",
                            "ünï",
                            "a",
                            " x ",
                        ])
                        .to_string(),
                );
            }
            let sep = *self.r.pick(&[",", ", ", " , "]);
            add("keywords", Val::Braced(kws.join(sep)));
        }
        if self.r.chance(0.12) {
            let n = self.r.range(1, 3);
            let mut os = Vec::new();
            for _ in 0..n {
                os.push(self.r.pick(OPTIONS).to_string());
            }
            let sep = *self.r.pick(&[",", ", "]);
            add("options", Val::Braced(os.join(sep)));
        }
        // Sorting/labelling helpers.
        if self.r.chance(0.04) {
            let t = self.title_text();
            add("sortkey", self.text_val(&t));
        }
        if self.r.chance(0.03) {
            let t = self.title_text();
            add("sorttitle", self.text_val(&t));
        }
        if self.r.chance(0.03) {
            let v = self.r.pick(&["mm", "zz", "0", "A", "ü"]).to_string();
            add("presort", Val::Braced(v));
        }
        if self.r.chance(0.03) {
            add(
                "sortyear",
                Val::Braced(self.r.range(1900, 2020).to_string()),
            );
        }
        if self.r.chance(0.05) {
            let v = self
                .r
                .pick(&["ABC", "Sm", "Ü", "{\\'E}x", "x1"])
                .to_string();
            add("label", Val::Braced(v));
        }
        if self.r.chance(0.05) {
            let v = self
                .r
                .pick(&["KpV", "GG", "\\emph{X}", "Ö", "a b"])
                .to_string();
            add("shorthand", Val::Braced(v));
        }
        if self.r.chance(0.08) {
            let v = self
                .r
                .pick(&[
                    "english",
                    "german",
                    "ngerman",
                    "french",
                    "american",
                    "british",
                    "russian",
                    "greek",
                    "spanish",
                    "unknownlang",
                    "German",
                ])
                .to_string();
            let name = *self.r.pick(&["langid", "hyphenation", "langid"]);
            add(name, Val::Braced(v));
        }
        // Verbatim and URI fields.
        if self.r.chance(0.12) {
            let v = self
                .r
                .pick(&[
                    "http://example.com/a b?c=d&e=%20f#frag",
                    "https://www.example.org/~user/über.html",
                    "http://例え.jp/ü",
                    "http://x.y/a\\_b",
                    "https://doi.org/10.1000/182",
                    "http://ex.com/{braces}",
                    "www.no-scheme.com",
                    "http://ex.com/%7Euser",
                    "",
                ])
                .to_string();
            add("url", Val::Braced(v));
        }
        if self.r.chance(0.1) {
            let v = self
                .r
                .pick(&[
                    "10.1000/xyz_123%",
                    "10.1007/978-3-540-74958-5\\_27",
                    "10.1002/(SICI)1097-0258(19980815/30)17:15/16<1661::AID-SIM968>3.0.CO;2-2",
                    "doi:10.1/x",
                ])
                .to_string();
            add("doi", Val::Braced(v));
        }
        if self.r.chance(0.05) {
            add("eprint", Val::Braced("1234.5678v2".into()));
            if self.r.chance(0.6) {
                add("eprinttype", Val::Braced("arxiv".into()));
            }
            if self.r.chance(0.3) {
                add("eprintclass", Val::Braced("cs.DL".into()));
            }
        }
        if self.r.chance(0.04) {
            let v = self
                .r
                .pick(&["a\\_b.pdf:PDF", "C:\\dir\\x.pdf", "%x#y~z", "\\foo{x}"])
                .to_string();
            let name = *self.r.pick(&["file", "verba", "verbb", "pdf"]);
            add(name, Val::Braced(v));
        }
        if self.r.chance(0.04) {
            let v = self
                .r
                .pick(&[
                    "978-3-16-148410-0",
                    "0-306-40615-2",
                    "9783161484100",
                    "123",
                    "979-10-90636-07-1",
                ])
                .to_string();
            add("isbn", Val::Braced(v));
        }
        if self.r.chance(0.03) {
            add("issn", Val::Braced("0317-8471".into()));
        }
        // Misc literal fields.
        if self.r.chance(0.15) {
            let t = self.title_text();
            let name = *self
                .r
                .pick(&["note", "addendum", "usera", "howpublished", "abstract"]);
            add(name, self.text_val(&t));
        }
        if self.r.chance(0.04) {
            let p = self
                .r
                .pick(&["inpress", "submitted", "forthcoming", "prepublished"]);
            add("pubstate", Val::Braced(p.to_string()));
        }
        if self.r.chance(0.04) {
            let f = self
                .r
                .pick(&["foo", "Bar", "customfield", "annote", "key", "crossreF"]);
            add(f, Val::Braced("unknown".into()));
        }
        // Annotations.
        if self.r.chance(0.04) {
            let (f, v) = *self.r.pick(&[
                ("author+an", "1=corresponding; 2:family=highlight"),
                ("title+an", "=emph"),
                ("author+an:role", "1=\"lead\""),
                ("location+an", "2=x"),
            ]);
            add(f, Val::Braced(v.to_string()));
        }

        // Entry graph.
        let others: Vec<usize> = (0..self.keys.len()).filter(|&j| j != i).collect();
        if !others.is_empty() {
            let contained = CONTAINED.contains(&ty.as_str());
            if self.r.chance(if contained { 0.5 } else { 0.1 }) {
                let parents: Vec<usize> = others
                    .iter()
                    .copied()
                    .filter(|&j| CONTAINERS.contains(&self.types[j].as_str()))
                    .collect();
                let j = if !parents.is_empty() && self.r.chance(0.7) {
                    *self.r.pick(&parents)
                } else {
                    *self.r.pick(&others)
                };
                let k = self.keys[j].clone();
                add("crossref", Val::Braced(k));
            }
            if self.r.chance(0.07) {
                let k = self.keys[*self.r.pick(&others)].clone();
                add("xref", Val::Braced(k));
            }
            if self.r.chance(0.07) {
                let mut ks = vec![self.keys[*self.r.pick(&others)].clone()];
                if self.r.chance(0.3) {
                    ks.push(self.keys[*self.r.pick(&others)].clone());
                }
                add("related", Val::Braced(ks.join(",")));
                if self.r.chance(0.8) {
                    let t = self.r.pick(&[
                        "translationof",
                        "reprintof",
                        "multivolume",
                        "origpubin",
                        "default",
                        "unknowntype",
                    ]);
                    add("relatedtype", Val::Braced(t.to_string()));
                }
                if self.r.chance(0.2) {
                    add("relatedstring", Val::Braced("see also".into()));
                }
                if self.r.chance(0.2) {
                    let o = self
                        .r
                        .pick(&["dataonly", "skipbib=false", "useprefix=true"]);
                    add("relatedoptions", Val::Braced(o.to_string()));
                }
            }
        }
        if self.r.chance(0.01) {
            add("crossref", Val::Braced(key.clone()));
        } else if self.r.chance(0.02) {
            add("crossref", Val::Braced("missingparent".into()));
        }
        if !self.xdata_keys.is_empty() && self.r.chance(0.35) {
            let mut ks = vec![self.r.pick(&self.xdata_keys).clone()];
            if self.r.chance(0.2) {
                ks.push(self.r.pick(&self.xdata_keys).clone());
            }
            if self.r.chance(0.03) {
                ks.push("nosuchxdata".into());
            }
            add("xdata", Val::Braced(ks.join(", ")));
        }
        if !self.xdata_keys.is_empty() && self.r.chance(0.15) {
            // Granular XDATA references: xdata=<key>-<field>[-<n>].
            let xi = self.r.below(self.xdata_keys.len());
            let xk = self.xdata_keys[xi].clone();
            let na = self.xdata_authors[xi];
            match self.r.below(5) {
                0 if na > 0 => {
                    let r = if self.r.chance(0.3) {
                        String::new()
                    } else {
                        format!("-{}", self.r.range(1, na as i64))
                    };
                    let v = if self.r.chance(0.7) {
                        format!("{} and xdata={xk}-author{r}", self.one_name())
                    } else {
                        format!("xdata={xk}-author{r}")
                    };
                    add("author", Val::Braced(v))
                }
                1 => add("publisher", Val::Braced(format!("xdata={xk}-publisher"))),
                2 => add("title", Val::Braced(format!("xdata={xk}-note"))),
                3 => add(
                    "location",
                    Val::Braced(format!(
                        "xdata={xk}-location-{} and Berlin",
                        self.r.range(1, 2)
                    )),
                ),
                _ => add("date", Val::Braced(format!("xdata={xk}-date"))),
            }
        }
        if self.r.chance(0.07) {
            let a = format!("alias{}", i + 1);
            self.aliases.push(a.clone());
            let v = if self.r.chance(0.3) {
                let b = format!("{a}b");
                self.aliases.push(b.clone());
                format!("{a}, {b}")
            } else {
                a
            };
            add("ids", Val::Braced(v));
        }
        if ty == "set" || self.r.chance(0.01) {
            let k = self.keys[self.r.below(self.keys.len())].clone();
            add("entryset", Val::Braced(k));
        }

        // Field-level oddities.
        if !fields.is_empty() && self.r.chance(0.02) {
            let f = fields[self.r.below(fields.len())].clone();
            fields.push(f);
        }
        if self.r.chance(0.02) {
            fields.push(Field {
                name: "title".into(),
                value: Val::Braced(String::new()),
            });
        }
        if self.r.chance(0.3) {
            self.r.shuffle(&mut fields);
        }
        let ty = match self.r.below(40) {
            0 => ty.to_uppercase(),
            1 => {
                let mut c = ty.chars();
                match c.next() {
                    Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                    None => ty,
                }
            }
            _ => ty,
        };
        Entry {
            ty,
            key,
            fields,
            parens: self.r.chance(0.03),
        }
    }

    fn xdata_entry(&mut self, i: usize) -> Entry {
        let key = self.xdata_keys[i].clone();
        let mut fields = Vec::new();
        let mut nauthors = 0;
        if self.r.chance(0.6) {
            // Plain names only: an XDATA entry skipped for a bad name crashes
            // biber when it is referenced (cf. BB-133).
            nauthors = self.r.range(1, 3) as usize;
            let v = (0..nauthors)
                .map(|_| {
                    let p = &self.persons[self.r.below(self.persons.len())];
                    format!("{}, {}", p.family, p.given)
                })
                .collect::<Vec<_>>()
                .join(" and ");
            fields.push(Field {
                name: "author".into(),
                value: Val::Braced(v),
            });
        }
        self.xdata_authors.push(nauthors);
        if self.r.chance(0.6) {
            fields.push(Field {
                name: "publisher".into(),
                value: Val::Braced(self.literal_list(&["Springer", "Elsevier", "{A and B}"])),
            });
        }
        if self.r.chance(0.5) {
            fields.push(Field {
                name: "location".into(),
                value: Val::Braced(self.literal_list(&["Berlin", "Paris", "Wien"])),
            });
        }
        if self.r.chance(0.5) {
            let d = self.date();
            fields.push(Field {
                name: "date".into(),
                value: Val::Braced(d),
            });
        }
        if self.r.chance(0.4) {
            fields.push(Field {
                name: "note".into(),
                value: Val::Braced(self.title_text()),
            });
        }
        // Chains and (rarely) cycles.
        if self.xdata_keys.len() > 1 && self.r.chance(0.25) {
            let other = self.xdata_keys[(i + 1) % self.xdata_keys.len()].clone();
            fields.push(Field {
                name: "xdata".into(),
                value: Val::Braced(other),
            });
        }
        if self.r.chance(0.02) {
            fields.push(Field {
                name: "xdata".into(),
                value: Val::Braced(key.clone()),
            });
        }
        Entry {
            ty: if self.r.chance(0.9) { "xdata" } else { "XDATA" }.into(),
            key,
            fields,
            parens: false,
        }
    }

    fn cites(&mut self) -> (Vec<Cite>, bool) {
        let nocite_all = self.r.chance(0.65);
        let mut cites = Vec::new();
        let mut pool: Vec<String> = self.keys.clone();
        pool.extend(self.xdata_keys.iter().cloned());
        let n = if nocite_all {
            self.r.range(0, 3) as usize
        } else {
            self.r.range(1, pool.len().max(1) as i64) as usize
        };
        for _ in 0..n {
            let k = match self.r.below(20) {
                0 if !self.aliases.is_empty() => self.r.pick(&self.aliases).clone(),
                1 => "missingkey".to_string(),
                2 if !pool.is_empty() => self.r.pick(&pool).to_uppercase(),
                _ if !pool.is_empty() => self.r.pick(&pool).clone(),
                _ => "missingkey".to_string(),
            };
            if cites.iter().any(|c: &Cite| c.key == k) && self.r.chance(0.8) {
                continue;
            }
            cites.push(Cite {
                key: k,
                nocite: self.r.chance(0.1),
            });
        }
        (cites, nocite_all)
    }

    // ---- values -------------------------------------------------------------

    /// Wrap text as a field value: braces mostly, quotes sometimes.
    fn text_val(&mut self, s: &str) -> Val {
        if self.r.chance(0.12) && quotable(s) {
            Val::Quoted(s.to_string())
        } else {
            Val::Braced(s.to_string())
        }
    }

    fn maybe_macro_val(&mut self, s: &str) -> Val {
        if !self.macros.is_empty() && self.r.chance(0.15) {
            let m = self.r.pick(&self.macros).clone();
            if self.r.chance(0.5) {
                Val::Bare(m)
            } else {
                Val::Concat(vec![Val::Bare(m), Val::Braced(format!(": {s}"))])
            }
        } else if self.r.chance(0.01) {
            Val::Bare("undefinedmacro".into())
        } else {
            self.text_val(s)
        }
    }

    fn word(&mut self) -> String {
        let w = self.r.pick(WORDS).to_string();
        self.decorate(&w)
    }

    /// Randomly LaTeX-ify or Unicode-ify one word.
    fn decorate(&mut self, w: &str) -> String {
        if w.starts_with('{') || !self.r.chance(0.12) {
            return w.to_string();
        }
        match self.r.below(14) {
            0 => format!("{{\\'e}}{w}"),
            1 => format!("{w}\\'{{e}}"),
            2 => format!("\\\"o{{}}{w}"),
            3 => format!("{{\\ss}}{w}"),
            4 => format!("\\c{{c}}{w}"),
            5 => format!("{w}\\v{{s}}"),
            6 => format!("\\emph{{{w}}}"),
            7 => format!("\\textbf{{{w}}}"),
            8 => format!("${w}^2$"),
            9 => format!("{w}e\u{301}"),
            10 => format!("{{\\o}}{w}{{\\aa}}"),
            11 => format!("\\H{{o}}\\k{{a}}\\u{{g}}\\={{a}}\\.{{z}}\\r{{a}}{w}"),
            12 => format!("{{\\i}}\\'{{\\i}}{w}"),
            _ => format!("\\mkbibquote{{{w}}}"),
        }
    }

    fn title_text(&mut self) -> String {
        if self.r.chance(0.02) {
            return self
                .r
                .pick(&["", " ", "{}", "x", "{The} {Book}", "\\relax"])
                .to_string();
        }
        let n = self.r.range(1, 6) as usize;
        let mut ws = Vec::new();
        for _ in 0..n {
            ws.push(self.word());
        }
        if self.r.chance(0.2) {
            let a = *self
                .r
                .pick(&["The", "A", "An", "Der", "Die", "La", "Le", "{The}"]);
            ws.insert(0, a.to_string());
        }
        let mut t = ws.join(" ");
        if let Some(first) = t.get(..1) {
            if first.chars().all(|c| c.is_ascii_lowercase()) && self.r.chance(0.6) {
                t = first.to_uppercase() + &t[1..];
            }
        }
        match self.r.below(30) {
            0 => t.push_str(" -- Part II"),
            1 => t.push_str("---``quoted''"),
            2 => t.push_str(": \\& \\% \\$ \\#"),
            3 => t.push_str(" $\\alpha+\\beta$"),
            4 => t.push_str("~\\cite{x}"),
            5 => t.push_str(" \\textsuperscript{th}"),
            6 => t.push_str(" \\unknownmacro{x}"),
            7 => t.push_str("\u{a0}\u{200b}nbsp"),
            8 => t.push_str(" 🙂"),
            9 => t.push_str(" עברית"),
            10 => t.push_str(", {\\'{\\i}}"),
            11 => t.push_str("\n    continued\tline"),
            12 => t.push_str(" 50% {\\%} {\\\\} \\\\"),
            13 => t.push_str(" @ sign"),
            14 => t.push_str(" \\url{http://x.org/~a}"),
            15 => t.push_str("  double  space "),
            16 => t.push_str(" {\\noopsort{a}}z"),
            17 => t.push_str(" \\`a\\^o\\~n\\=e\\.z\\'y\\\"y"),
            18 => t.push_str(" {\\dj} {\\l} {\\ae} {\\OE} {\\ng} {\\th}"),
            _ => {}
        }
        t
    }

    fn literal_list(&mut self, pool: &[&str]) -> String {
        let n = *self.r.weighted(&[(10, 1usize), (4, 2), (1, 3)]);
        let mut xs = Vec::new();
        for _ in 0..n {
            xs.push(self.r.pick(pool).to_string());
        }
        if self.r.chance(0.05) {
            xs.push("others".into());
        }
        xs.join(" and ")
    }

    // ---- names --------------------------------------------------------------

    fn name_list(&mut self) -> String {
        let n = *self
            .r
            .weighted(&[(30, 1usize), (25, 2), (15, 3), (8, 4), (4, 5), (3, 7)]);
        let mut names: Vec<String> = Vec::new();
        for _ in 0..n {
            names.push(self.one_name());
        }
        if self.r.chance(0.08) {
            names.push(
                if self.r.chance(0.9) {
                    "others"
                } else {
                    "Others"
                }
                .to_string(),
            );
        }
        if self.r.chance(0.04) {
            let o = self
                .r
                .pick(&[
                    "useprefix=true",
                    "useprefix",
                    "uniquelist=false",
                    "uniquename=false",
                    "sortingnamekeytemplatename=global",
                    "foo=bar",
                ])
                .to_string();
            if self.r.chance(0.5) {
                names.insert(0, o);
            } else {
                names.push(o);
            }
        }
        if self.r.chance(0.01) {
            names.push(String::new()); // consecutive "and"
        }
        let mut out = String::new();
        for (i, name) in names.iter().enumerate() {
            if i > 0 {
                out.push_str(self.r.weighted(&[
                    (40, " and "),
                    (1, " AND "),
                    (1, "\n    and "),
                    (1, " and\t"),
                    (1, " And "),
                ]));
            }
            out.push_str(name);
        }
        out
    }

    fn one_name(&mut self) -> String {
        let p = if self.r.chance(0.7) {
            let p = &self.persons[self.r.below(self.persons.len())];
            Person {
                given: p.given.clone(),
                family: p.family.clone(),
                prefix: p.prefix.clone(),
                suffix: p.suffix.clone(),
            }
        } else {
            Person {
                given: self.r.pick(GIVEN).to_string(),
                family: self.r.pick(FAMILY).to_string(),
                prefix: self.r.chance(0.15).then(|| self.r.pick(PREFIX).to_string()),
                suffix: self.r.chance(0.05).then(|| self.r.pick(SUFFIX).to_string()),
            }
        };
        let given = if self.r.chance(0.2) {
            initials(&p.given)
        } else {
            p.given.clone()
        };
        match self.r.below(100) {
            0..=29 => match &p.prefix {
                Some(pre) => format!("{given} {pre} {}", p.family),
                None => format!("{given} {}", p.family),
            },
            30..=64 => match (&p.prefix, &p.suffix) {
                (Some(pre), Some(suf)) => format!("{pre} {}, {suf}, {given}", p.family),
                (Some(pre), None) => format!("{pre} {}, {given}", p.family),
                (None, Some(suf)) => format!("{}, {suf}, {given}", p.family),
                (None, None) => format!("{}, {given}", p.family),
            },
            65..=69 => self.r.pick(CORPORATE).to_string(),
            70..=72 => p.family.clone(),
            73..=82 => {
                // Extended name format.
                let mut parts = vec![
                    format!("family={}", quote_np(&mut self.r, &p.family)),
                    format!("given={}", quote_np(&mut self.r, &given)),
                ];
                if let Some(pre) = &p.prefix {
                    parts.push(format!("prefix={}", quote_np(&mut self.r, pre)));
                    if self.r.chance(0.5) {
                        parts.push(
                            self.r
                                .pick(&["useprefix=true", "useprefix=false", "useprefix"])
                                .to_string(),
                        );
                    }
                }
                if let Some(suf) = &p.suffix {
                    parts.push(format!("suffix={suf}"));
                }
                if self.r.chance(0.15) {
                    parts.push(format!("given-i={}", initials(&p.given)));
                }
                if self.r.chance(0.1) {
                    parts.push(
                        self.r
                            .pick(&[
                                "uniquename=false",
                                "useprefix=true",
                                "sortingnamekeytemplatename=global",
                                "foo=bar",
                            ])
                            .to_string(),
                    );
                }
                self.r.shuffle(&mut parts);
                parts.join(if self.r.chance(0.8) { ", " } else { "," })
            }
            83..=86 => match &p.suffix {
                Some(suf) => format!("{given} {} {suf}", p.family),
                None => format!("{given} {} Jr.", p.family),
            },
            87..=88 => format!("{}, {}", p.family.to_uppercase(), given.to_uppercase()),
            89..=90 => format!("{}, {}", p.family.to_lowercase(), given),
            _ => self
                .r
                .pick(&[
                    "",
                    "{}",
                    "Smith,",
                    "A, B, C, D",
                    "a,,b",
                    "{Smith, John}",
                    "John {von} Smith",
                    "von",
                    "John~Smith",
                    "J.~R. Smith",
                    "\\textsc{Smith}, John",
                    "Smith, {\\relax J}ohn",
                    "John Smith, Jr., III",
                    "Ludwig van Beethoven",
                    "{van} Gogh, Vincent",
                    "de la Fontaine, Jean",
                    "Jean de la Fontaine",
                    "{\\v{S}}im{\\'a}{\\v{c}}ek, Ji{\\v{r}}{\\'\\i}",
                    "ölund, x",
                    "Ǆemal Ǆukić",
                    "{\\relax Ch}ristopher Smith",
                    "Smith, J{\\\"o}rg",
                    "family=Smith",
                    "given=John",
                    "family={Smith, Jr.}, given=John",
                    "family=Doe, given={J. P.}, given-i={JP}",
                    "family=Doe, \"given=J, Jr\"",
                    "\"family=Doe\", given=Jane",
                    "family=\"Doe\", given=Jane",
                    "family=Doe,, given=Jane",
                    "family = Doe , given = Jane",
                    "FAMILY=Doe, Given=Jane",
                    "family=Doe, given=Jane, id=x1",
                    "family=Doe, given=Jane, prefix=von, useprefix=true, suffix=Jr",
                    "family=Doe, given=Jane, given-i=Ja",
                    "family=Doe, family=Roe",
                    "family=, given=Jane",
                    "family={Doe, Jane}",
                    "Doe, {J}ohn",
                    "{\\'E}mile Zola",
                    "Émile Zola",
                    "E\u{301}mile Zola",
                    "Ab. Cd Ef",
                    "x",
                ])
                .to_string(),
        }
    }

    // ---- dates --------------------------------------------------------------

    fn year(&mut self) -> i64 {
        if self.r.chance(0.6) {
            *self.r.pick(&self.years)
        } else {
            self.r.range(1000, 2030)
        }
    }

    fn date_point(&mut self) -> String {
        let y = self.year();
        let ys = match self.r.below(40) {
            0 => format!("-{:04}", self.r.range(1, 500)),
            1 => "0000".into(),
            2 => format!("Y{}", self.r.range(10000, 99999)),
            3 => format!("Y-{}", self.r.range(10000, 99999)),
            4 => format!("{:04}", self.r.range(1, 999)),
            _ => format!("{y:04}"),
        };
        let m = self.r.range(1, 12);
        let d = self.r.range(1, 28);
        let s = match self.r.below(20) {
            0..=7 => ys,
            8..=11 => format!("{ys}-{m:02}"),
            12..=15 => format!("{ys}-{m:02}-{d:02}"),
            16 => format!("{ys}-{}", self.r.range(21, 41)),
            17 => {
                let tz = *self.r.pick(&["", "Z", "+01:00", "-05:00", "+05:30"]);
                format!(
                    "{ys}-{m:02}-{d:02}T{:02}:{:02}:{:02}{tz}",
                    self.r.range(0, 23),
                    self.r.range(0, 59),
                    self.r.range(0, 59)
                )
            }
            18 => format!(
                "{ys}-{m:02}-{d:02}T{:02}:{:02}:{:02}.123",
                self.r.range(0, 23),
                self.r.range(0, 59),
                self.r.range(0, 59)
            ),
            _ => match self.r.below(5) {
                0 => format!("{}X", &format!("{y:04}")[..3]),
                1 => format!("{}XX", &format!("{y:04}")[..2]),
                2 => format!("{y:04}-XX"),
                3 => format!("{y:04}-{m:02}-XX"),
                _ => format!("{y:04}-XX-XX"),
            },
        };
        match self.r.below(25) {
            0 => format!("{s}?"),
            1 => format!("{s}~"),
            2 => format!("{s}%"),
            _ => s,
        }
    }

    fn date(&mut self) -> String {
        match self.r.below(30) {
            0..=17 => self.date_point(),
            18..=22 => {
                let a = self.date_point();
                let b = self.date_point();
                format!("{a}/{b}")
            }
            23 => format!("{}/", self.date_point()),
            24 => format!("/{}", self.date_point()),
            25 => format!("../{}", self.date_point()),
            26 => format!("{}/..", self.date_point()),
            27 => {
                // Non-Arabic digits.
                let y = format!("{:04}", self.year());
                let digits: &[char] = if self.r.chance(0.5) {
                    &ARABIC_INDIC
                } else {
                    &DEVANAGARI
                };
                y.chars()
                    .map(|c| digits[c.to_digit(10).unwrap_or(0) as usize])
                    .collect()
            }
            _ => self
                .r
                .pick(&[
                    "2001-13",
                    "2001-02-30",
                    "2001-1-1",
                    "Jan 2001",
                    "1984--1990",
                    "abc",
                    "20011",
                    "2001/2002/2003",
                    "c. 1900",
                    "[1900]",
                    "1900s",
                    " 2001 ",
                    "{2001}",
                    "",
                    "2001-02-03T25:00:00",
                    "1999–XX",
                    "1999-21/2000-24",
                    "-0044-03-15",
                    "0000/2000",
                    "2000/1990",
                ])
                .to_string(),
        }
    }

    fn year_value(&mut self) -> Val {
        let y = self.year();
        match self.r.below(30) {
            0..=19 => {
                if self.r.chance(0.3) {
                    Val::Bare(y.to_string())
                } else {
                    Val::Braced(y.to_string())
                }
            }
            20 => Val::Braced(format!("{y}a")),
            21 => Val::Braced(format!("c{y}")),
            22 => Val::Braced(format!("[{y}]")),
            23 => Val::Braced(format!("{y}--{}", y + 3)),
            24 => Val::Braced("MCMLXXXIV".into()),
            25 => Val::Braced("١٩٨٤".into()),
            26 => Val::Braced(String::new()),
            27 => Val::Braced("0".into()),
            28 => Val::Braced(format!("-{y}")),
            _ => Val::Braced("forthcoming".into()),
        }
    }

    fn month_value(&mut self) -> Val {
        match self.r.below(12) {
            0..=3 => Val::Braced(self.r.range(1, 12).to_string()),
            4 => Val::Braced(format!("{:02}", self.r.range(1, 12))),
            5..=6 => Val::Bare(
                self.r
                    .pick(&["jan", "feb", "mar", "apr", "may", "jun", "dec", "sep"])
                    .to_string(),
            ),
            7 => Val::Braced(
                self.r
                    .pick(&["January", "February", "Mar.", "{Feb}", "Sept", "mai"])
                    .to_string(),
            ),
            8 => Val::Braced(self.r.pick(&["13", "0", "", "1-2", "II"]).to_string()),
            9 => Val::Concat(vec![Val::Bare("jan".into()), Val::Quoted("~1".into())]),
            _ => Val::Bare(self.r.range(1, 12).to_string()),
        }
    }

    fn integerish(&mut self) -> Val {
        match self.r.below(12) {
            0..=6 => Val::Braced(self.r.range(1, 120).to_string()),
            7 => Val::Bare(self.r.range(1, 20).to_string()),
            _ => Val::Braced(
                self.r
                    .pick(&[
                        "III",
                        "1a",
                        "-1",
                        "0",
                        "2/3",
                        "1--2",
                        "S1",
                        "{\\relax 4}",
                        "٣",
                    ])
                    .to_string(),
            ),
        }
    }

    fn range_value(&mut self) -> String {
        let a = self.r.range(1, 500);
        let b = a + self.r.range(0, 60);
        match self.r.below(28) {
            0..=7 => format!("{a}--{b}"),
            8..=9 => format!("{a}-{b}"),
            10 => format!("{a}–{b}"),
            11 => format!("{a}—{b}"),
            12 => format!("{a}"),
            13 => format!("{a}--{}", b % 100),
            14 => format!("{a}-{}", b % 10),
            15 => "xii--xv".into(),
            16 => "XII-XV".into(),
            17 => "iv".into(),
            18 => format!("{a}--"),
            19 => format!("--{b}"),
            20 => format!("{a}, {b}--{}", b + 5),
            21 => format!("{a} -- {b}"),
            22 => format!("A{a}--A{b}"),
            23 => format!("S{a}-S{b}"),
            24 => format!("{a}+"),
            25 => format!("{a}-{b}-{}", b + 2),
            26 => format!("{{{a}--{b}}}"),
            _ => self
                .r
                .pick(&[
                    "pp. 1-10",
                    "1,3,5--7",
                    "",
                    "-",
                    "--",
                    "ix--12",
                    "e123",
                    "1:1--1:20",
                ])
                .to_string(),
        }
    }

    // ---- syntax errors ------------------------------------------------------

    fn junk(&mut self) -> String {
        let k = self.keys.first().cloned().unwrap_or_else(|| "k".into());
        self.r
            .pick(&[
                "@book{bad1, title={Unbalanced}".to_string(),
                "@book{bad2 title={Missing comma}}".to_string(),
                "@book{bad3, title={x}} }".to_string(),
                "@book{,title={no key}}".to_string(),
                "@book{bad 4, title={key with space}}".to_string(),
                "@book{bad5, title}".to_string(),
                "@book{bad6, title = \"x\" y}".to_string(),
                "@{".to_string(),
                "@book{bad7, title={a}, title={b}}".to_string(),
                format!("@misc{{{k}, title={{duplicate key}}}}"),
                "@book{bad8, author={Smith and and Jones}}".to_string(),
                "@book{bad9, title={x}".to_string(),
                "@string{a=\"x\", b=\"y\"}".to_string(),
                "@book{bad10, year=19 99}".to_string(),
                "@book{bad11, title={x}, ,}".to_string(),
                "stray } brace".to_string(),
                "@book{bad12, title={\"}}".to_string(),
            ])
            .clone()
    }
}

/// Wrap a name part for the extended name format when it needs it.
fn quote_np(r: &mut Rng, s: &str) -> String {
    if s.contains(',') || s.contains('=') || r.chance(0.2) {
        if s.starts_with('{') {
            s.to_string()
        } else {
            format!("{{{s}}}")
        }
    } else {
        s.to_string()
    }
}

/// "Jean-Paul" → "J.-P.", "Hans Peter" → "H. P.".
fn initials(given: &str) -> String {
    if given.starts_with('{') || given.starts_with('\\') {
        return given.to_string();
    }
    let mut out = String::new();
    for (i, w) in given.split(' ').enumerate() {
        if i > 0 {
            out.push(' ');
        }
        for (j, part) in w.split('-').enumerate() {
            if j > 0 {
                out.push('-');
            }
            if let Some(c) = part.chars().next() {
                out.push(c);
                if !part.ends_with('.') || part.chars().count() > 1 {
                    out.push('.');
                }
            }
        }
    }
    out
}

fn ascii_only(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

/// Whether `s` can be written as a `"…"` value (no `"` at brace depth 0).
fn quotable(s: &str) -> bool {
    let mut depth = 0i32;
    for c in s.chars() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            '"' if depth == 0 => return false,
            _ => {}
        }
    }
    depth == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic() {
        for i in 0..50 {
            assert_eq!(generate(7, i), generate(7, i));
        }
    }

    #[test]
    fn initials_work() {
        assert_eq!(initials("Jean-Paul"), "J.-P.");
        assert_eq!(initials("Hans Peter"), "H. P.");
    }
}
