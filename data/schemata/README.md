# RELAX NG schemas

`bcf.rng` (the biblatex control file) and `config.rng` (the biber config file) are the schemas biber 2.22 validates against with `--validate-control` and `--validate-config` (docs/compat.md DC-033). bibers compiles them into the binary (`bibers-core::xmlcheck`) and validates with the `relaxng` crate.

- **Provenance:** copied verbatim from biber 2.22 (`vendor/biber/data/schemata/`, tag `v2.22`). Byte-identical to the copies bundled in the TeX Live 2026 biber 2.22 binary.
- **sha256:** `bcf.rng` `c985e8ee706e9c365faac1a09a054c1b44811a2bbad63cf75f7b3b104f9f0a1b`, `config.rng` `ecac183c528a8c5a1752fe9a58d93a4c663dd915ca7ffb29e1bf573a083ee532`.
- **Licence:** Artistic License 2.0 (part of biber).
- **Updating:** when following a newer biber release, replace the files with that release's copies and update this README.
