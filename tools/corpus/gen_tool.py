#!/usr/bin/env python3
"""Generate the `tool/` corpus suite: tool mode (`--tool`) and bibtex output
(`--output-format=bibtex`) with the `--output-*` options, alone and combined.

Inputs: biber's t/tdata fixtures (tool.bib, examples.bib, the tool config
files, encoding and section fixtures), biblatex-examples.bib (shared), and
the synthetic files below (`tooldata.bib`: dates, seasons, unspecified
dates, annotations, macros, preambles, comments; config files with source
maps and output options).

Usage: python3 tools/corpus/gen_tool.py && cargo xtask golden tool/
"""
from __future__ import annotations

from corpuslib import CORPUS, TDATA, ensure_shared_examples_bib, write_case

SUITE = CORPUS / "tool"

TOOLDATA = r"""@string{pub = "Publisher Name"}
@string{loc = {Berlin}}
@string{unused = {Never used}}
@STRING{Jnl = {Journal of Things}}

@preamble{"\newcommand{\noop}[1]{}"}

@comment{A plain comment}

@book{season1,
  author = {Doe, John},
  title = {Spring book},
  date = {2020-21},
  publisher = pub,
}

@book{season2,
  author = {Doe, John and von Neumann, Jr., John},
  title = {Southern winter?},
  date = {2020-32},
  location = loc,
}

@book{unspec1,
  author = {Roe, Richard},
  title = {Decade},
  date = {199X},
}

@book{unspec2,
  editor = {{Barnes and Noble}},
  title = {Century},
  date = {19XX},
}

@book{unspec3,
  title = {Month unspecified},
  date = {1999-XX},
}

@book{unspec4,
  title = {Day unspecified},
  date = {1999-01-XX},
}

@book{unspec5,
  title = {Day in year},
  date = {1999-XX-XX},
}

@book{midnight,
  title = {Midnight},
  date = {2020-01-01T00:00:00},
  eventdate = {2020-01-01T10:20:30+02:00},
}

@book{year0,
  title = {Year zero},
  date = {0000},
  volume = {0},
  number = {0},
}

@book{circa,
  title = {Circa},
  date = {1850~/1860?},
  origdate = {1700%},
}

@book{era,
  title = {BCE},
  date = {-0044-03-15},
}

@book{open,
  title = {Open range},
  date = {1990/},
  pages = {10-},
}

@book{legacy,
  title = {Legacy},
  year = {1995},
  month = {mar},
  pages = {1--10, 20-30},
}

@book{legacy2,
  title = {Legacy numeric month},
  year = {1996},
  month = {11},
}

@book{legacyday,
  title = {Legacy with day},
  date = {1995-03-04},
}

@article{ann,
  author = {Alpha, A. and Beta, B. and others},
  author+an = {1=corresponding; 2:family="Lit"},
  title = {Annotated},
  title+an:alt = {=alternative},
  journaltitle = Jnl,
  keywords = {a, b,c},
  url = {http://example.org/a?b=c},
}

@online{verb,
  title = {Ünïcödé “quotes” – dash ß},
  url = {http://example.org/caf\'e},
  urldate = {2020-05-06},
  note = {x}
}

@comment{jabref-meta: databaseType:biblatex;}

@inproceedings{opts,
  author = {Gamma, G. and Delta, D.},
  author = {Ignored, Dup},
  options = {skipbib},
  title = {Options and a very long title that will need to be wrapped when the output is wrapped at forty columns},
  booktitle = {Proceedings},
  month = jan,
  year = {2001},
  crossref = {proc},
}

@proceedings{proc,
  title = {The Proceedings},
  editor = {Epsilon, E.},
  year = {2001},
  publisher = pub,
}
"""

TOOLDATA2 = r"""@comment{Second file comment}
@string{loc = {Paris}}
@misc{second1,
  author = {Zeta, Z.},
  title = {From the second file},
  location = loc,
  related = {season1},
}
@set{set1,
  entryset = {season1,season2},
}
"""

SMAP_CONF = r"""<?xml version="1.0" encoding="UTF-8"?>
<config>
  <output_fieldcase>lower</output_fieldcase>
  <output_align>1</output_align>
  <output_indent>3</output_indent>
  <sourcemap>
    <maps datatype="bibtex">
      <map>
        <map_step map_field_set="note" map_field_value="mapped note"/>
      </map>
      <map>
        <map_step map_type_source="book" map_type_target="collection" map_final="1"/>
      </map>
      <map>
        <map_step map_field_source="title" map_match="(\w+)" map_replace="[$1]"/>
      </map>
    </maps>
  </sourcemap>
</config>
"""

SORT_CONF = r"""<?xml version="1.0" encoding="UTF-8"?>
<config>
  <sortingtemplate name="tool">
    <sort order="1">
      <sortitem order="1">title</sortitem>
    </sort>
  </sortingtemplate>
</config>
"""

TB = TDATA / "tool.bib"


def case(name, args, outputs, files, desc, log=None, shared=None, tags=(), allow=()):
    spec = {
        "description": desc,
        "source": "tools/corpus/gen_tool.py",
        "args": ["--noconf"] + args,
        "outputs": outputs,
        "tags": ["suite:tool", "feature:tool"] + list(tags),
    }
    if shared:
        spec["shared"] = shared
    if allow:
        spec["allow"] = list(allow)
    if log:
        spec["log"] = log
    write_case(SUITE / name, spec, files)


def tool_case(name, opts, desc, files=None, data=("tooldata.bib",), out=None, **kw):
    files = files or {"tooldata.bib": TOOLDATA}
    out = out or data[0].rsplit(".", 1)[0] + "_bibertool.bib"
    case(name, ["--tool"] + opts + list(data), [out], files, desc,
         log=kw.pop("log", data[0] + ".blg"), **kw)


def main():
    ensure_shared_examples_bib()
    SUITE.mkdir(parents=True, exist_ok=True)
    tb = {"tool.bib": TB}
    # Tool mode over tool.bib, one option at a time.
    single = [
        ("default", [], "tool mode defaults"),
        ("align", ["--output-align"], "--output-align"),
        ("indent-4", ["--output-indent=4"], "--output-indent=4"),
        ("indent-tabs", ["--output-indent=2t", "--output-align"], "tab indent, aligned"),
        ("indent-0", ["--output-indent=0"], "--output-indent=0"),
        ("fieldcase-lower", ["--output-fieldcase=lower"], "--output-fieldcase=lower"),
        ("fieldcase-title", ["--output-fieldcase=title"], "--output-fieldcase=title"),
        ("field-order", ["--output-field-order=title,dates,names,author,lists,keywords"],
         "custom --output-field-order (classes and fields)"),
        ("field-order-empty", ["--output-field-order=,"], "empty --output-field-order"),
        ("field-replace", ["--output-field-replace=location:address, journaltitle:journal,title:"],
         "--output-field-replace (one malformed pair)"),
        ("legacy-dates", ["--output-legacy-dates"], "--output-legacy-dates"),
        ("xname", ["--output-xname"], "--output-xname"),
        ("xname-sep", ["--output-xname", "--output-xnamesep=:"], "--output-xname with --output-xnamesep"),
        ("listsep", ["--output-listsep=&", "--output-namesep=AND"], "--output-listsep/--output-namesep"),
        ("xdata-markers", ["--output-xdatamarker=xd", "--output-xdatasep=+", "--output-xnamesep=:"],
         "XDATA reference output markers"),
        ("annotation-markers", ["--output-annotation-marker=!a", "--output-named-annotation-marker=@"],
         "annotation output markers"),
        ("no-macrodefs", ["--output-no-macrodefs"], "--output-no-macrodefs"),
        ("all-macrodefs", ["--output-all-macrodefs"], "--output-all-macrodefs"),
        ("resolve", ["--output-resolve"], "--output-resolve"),
        ("resolve-sets", ["--output-resolve-sets"], "--output-resolve-sets"),
        ("resolve-crossrefs", ["--output-resolve-crossrefs"], "--output-resolve-crossrefs"),
        ("tool-resolve", ["--tool-resolve", "--tool-align", "--tool-fieldcase=lower", "--tool-indent=1"],
         "legacy --tool-* aliases"),
        ("strip-comments", ["--strip-comments"], "--strip-comments"),
        ("wraplines", ["--wraplines=40", "--output-align"], "--wraplines=40 aligned"),
        ("wraplines-tabs", ["-w", "--output-indent=1t"], "-w (80) with tab indent"),
        ("safechars", ["--output-safechars"], "--output-safechars"),
        ("safecharsset-full", ["--output-safechars", "--output-safecharsset=full"],
         "--output-safechars with the full set"),
        ("latin1", ["--output-encoding=latin1"], "--output-encoding=latin1 (BB-048 on '?')"),
        ("ascii", ["--output-encoding=ascii"], "--output-encoding=ascii (forces safechars)"),
        ("utf8-alias", ["--output-encoding=utf8"], "--output-encoding=utf8 (not normalised in tool mode)"),
        ("noremove-missing", ["--tool-noremove-missing-dependants"], "--tool-noremove-missing-dependants"),
        ("mincrossrefs", ["--mincrossrefs=1"], "--mincrossrefs=1"),
        ("nostdmacros", ["--nostdmacros", "--output-legacy-dates"], "--nostdmacros with legacy dates"),
        ("isbn", ["--isbn13", "--isbn-normalise"], "ISBN options"),
    ]
    for name, opts, desc in single:
        tool_case(f"tool-{name}", opts, f"tool mode over t/tdata/tool.bib: {desc}",
                  files=tb, data=("tool.bib",))
        tool_case(f"data-{name}", opts, f"tool mode over a synthetic .bib: {desc}")
    # Combinations.
    combos = [
        ("combo-1", ["--output-align", "--output-indent=4", "--output-fieldcase=lower", "--output-resolve",
                     "--output-xname", "--output-legacy-dates"]),
        ("combo-2", ["--output-field-order=names,title", "--output-listsep=AND", "--output-namesep=AND",
                     "--output-all-macrodefs", "--output-xnamesep=:", "--output-xname", "--wraplines=50"]),
        ("combo-3", ["--output-safechars", "--output-indent=2t", "--wraplines=60", "--strip-comments",
                     "--output-fieldcase=title", "--output-align"]),
        ("combo-4", ["--output-encoding=latin1", "--output-no-macrodefs", "--output-annotation-marker=+ann",
                     "--output-xdatamarker=xd", "--output-field-replace=journaltitle:journal,location:address",
                     "--output-resolve-crossrefs"]),
        ("combo-5", ["--output-resolve-xdata", "--output-resolve-sets", "--output-align",
                     "--output-field-order=options,abstract,dates,names", "--output-legacy-dates",
                     "--output-fieldcase=lower"]),
    ]
    for name, opts in combos:
        tool_case(f"tool-{name}", opts, "tool mode over t/tdata/tool.bib: " + " ".join(opts),
                  files=tb, data=("tool.bib",))
        tool_case(f"data-{name}", opts, "tool mode over a synthetic .bib: " + " ".join(opts))
    # Several data files.
    two = {"tooldata.bib": TOOLDATA, "second.bib": TOOLDATA2}
    tool_case("multi-files", [], "tool mode with two data files (only the last file's comments)",
              files=two, data=("tooldata.bib", "second.bib"))
    tool_case("multi-files-rev", ["--output-align"], "tool mode with two data files, reversed",
              files=two, data=("second.bib", "tooldata.bib"))
    tool_case("multi-tool-data", ["--output-resolve"], "tool.bib and a synthetic file together",
              files={"tool.bib": TB, "tooldata.bib": TOOLDATA}, data=("tool.bib", "tooldata.bib"))
    # Documented deliberate differences (expected-bibers/, `cargo xtask bless`).
    case("dc-007-fieldcase-upper", ["--tool", "--output-fieldcase=Upper", "tooldata.bib"],
         ["tooldata_bibertool.bib"], {"tooldata.bib": TOOLDATA},
         "--output-fieldcase=Upper: biber crashes (BB-008), bibers accepts it (DC-007)",
         log="tooldata.bib.blg", allow=("DC-007",))
    case("dc-011-no-extension", ["--tool", "refs"], ["refs_bibertool.bib", "refs"],
         {"refs": TOOLDATA},
         "tool mode on a data file without extension: biber overwrites it (BB-013), bibers writes refs_bibertool.bib (DC-011)",
         log="refs.blg", allow=("DC-011",))
    case("dc-055-tool-config", ["--tool-config"], ["@stdout"], {},
         "--tool-config: biber prints the path of biber-tool.conf, bibers its built-in copy (DC-055)",
         allow=("DC-055",))
    # Output file options.
    case("output-file-stdout", ["--tool", "--onlylog", "-O", "-", "tooldata.bib"], ["@stdout"],
         {"tooldata.bib": TOOLDATA}, "tool mode with --output-file - (stdout)", log="tooldata.bib.blg")
    case("output-file-named", ["--tool", "--output-file=out.bib", "tooldata.bib"], ["out.bib"],
         {"tooldata.bib": TOOLDATA}, "tool mode with --output-file", log="tooldata.bib.blg")
    case("output-file-inplace", ["--tool", "--output-file=tooldata.bib", "tooldata.bib"], ["tooldata.bib"],
         {"tooldata.bib": TOOLDATA}, "tool mode writing over its own input", log="tooldata.bib.blg")
    case("output-format-default", ["--tool", "--output-format=bibtex", "--logfile=x", "tooldata.bib"],
         ["tooldata_bibertool.bib"], {"tooldata.bib": TOOLDATA}, "explicit format and --logfile", log="x.blg")
    # Config files in tool mode.
    tool_case("configfile-smap", ["--configfile=smap.conf"],
              "tool mode with a config file setting output options and user source maps",
              files={"tooldata.bib": TOOLDATA, "smap.conf": SMAP_CONF})
    tool_case("configfile-sort", ["--configfile=sort.conf"], "tool mode with a user 'tool' sorting template",
              files={"tooldata.bib": TOOLDATA, "sort.conf": SORT_CONF})
    tool_case("configfile-testsort", ["--configfile=tool-testsort.conf"],
              "tool mode with t/tdata/tool-testsort.conf, all defaults",
              files={"tool.bib": TB, "tool-testsort.conf": TDATA / "tool-testsort.conf"}, data=("tool.bib",))
    # tool-config.t: tool mode with tool-testconfig.conf (mincrossrefs=5, a bad
    # sort item, data model additions).
    tool_case("tool-config-t", ["--configfile=tool-testconfig.conf"],
              "biber t/tool-config.t: tool mode with t/tdata/tool-testconfig.conf",
              files={"tool.bib": TB, "tool-testconfig.conf": TDATA / "tool-testconfig.conf"},
              data=("tool.bib",), tags=("t:tool-config",))
    # Input encodings.
    for n, enc in (("encoding2", None), ("encoding3", "latin1"), ("encoding5", "latin1")):
        opts = [f"--input-encoding={enc}"] if enc else []
        tool_case(f"input-{n}", opts, f"tool mode over t/tdata/{n}.bib",
                  files={f"{n}.bib": TDATA / f"{n}.bib"}, data=(f"{n}.bib",))
        tool_case(f"input-{n}-latin1-out", opts + ["--output-encoding=latin1"],
                  f"tool mode over t/tdata/{n}.bib, latin1 output",
                  files={f"{n}.bib": TDATA / f"{n}.bib"}, data=(f"{n}.bib",))
    # Other fixtures.
    for n in ("annotations", "xdata", "examples", "sections1", "bibtex-aliases"):
        tool_case(f"fixture-{n}", [], f"tool mode over t/tdata/{n}.bib",
                  files={f"{n}.bib": TDATA / f"{n}.bib"}, data=(f"{n}.bib",))
    # (xdata.bib with --output-resolve-xdata hits an XDATA cycle, whose report
    # is nondeterministic in biber: DC-017.)
    for name, opts in (("examples", []), ("examples-resolve", ["--output-resolve", "--output-align"]),
                       ("examples-xname", ["--output-xname", "--output-legacy-dates"])):
        case(f"biblatex-{name}", ["--tool"] + opts + ["biblatex-examples.bib"],
             ["biblatex-examples_bibertool.bib"], {}, "tool mode over biblatex-examples.bib " + " ".join(opts),
             log="biblatex-examples.bib.blg", shared=["biblatex-examples.bib"])
    # Non-tool bibtex output from a .bcf.
    bo = {"bibtex-output.bcf": TDATA / "bibtex-output.bcf", "examples.bib": TDATA / "examples.bib",
          "tool.bib": TB}
    for name, opts in (("default", []), ("legacy-xname", ["--output-legacy-dates", "--output-xname"]),
                       ("resolve", ["--output-resolve", "--output-fieldcase=title"]),
                       ("safechars", ["--output-safechars", "--output-indent=3"]),
                       ("wraplines", ["-w", "50", "--output-align"])):
        case(f"bcf-{name}", ["--output-format=bibtex"] + opts + ["bibtex-output.bcf"],
             ["bibtex-output_biber.bib"], bo, "--output-format=bibtex from t/tdata/bibtex-output.bcf " + " ".join(opts),
             log="bibtex-output.blg")
    case("bcf-output-file", ["--output-format=bibtex", "--output-file=x.bib", "bibtex-output.bcf"],
         ["x.bib"], bo, "--output-format=bibtex with --output-file", log="bibtex-output.blg")
    sec = {"sections.bcf": TDATA / "sections.bcf"}
    for i in range(1, 5):
        sec[f"sections{i}.bib"] = TDATA / f"sections{i}.bib"
    case("bcf-sections", ["--output-format=bibtex", "sections.bcf"], ["sections_biber.bib"], sec,
         "--output-format=bibtex from a multi-section .bcf (t/tdata/sections.bcf)", log="sections.blg")
    fb = {"full-bibtex.bcf": TDATA / "full-bibtex.bcf", "examples.bib": TDATA / "examples.bib"}
    case("bcf-full-bibtex-stdout", ["--output-format=bibtex", "--onlylog", "--output-file=-", "full-bibtex.bcf"],
         ["@stdout"], fb, "--output-format=bibtex to stdout", log="full-bibtex.blg")


if __name__ == "__main__":
    main()
