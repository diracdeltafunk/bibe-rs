# ISBN range data

`RangeMessage.xml` is the ISBN International range file, used for ISBN validation, ISBN-10/13 conversion and hyphenation (`--isbn-normalise`).

- **Provenance:** copied verbatim from the CPAN distribution Business-ISBN-Data **20260218.001**. That is the version bundled in the TeX Live 2026 biber 2.22 binary, and the two files are byte-identical. MessageDate: Wed, 18 Feb 2026 04:26:53 GMT.
- **sha256:** `485c969a004640faafe6d894ff419d8f66e693efa0c5bf98a8a89d645fa9e8ab`
- **Licence:** Business-ISBN-Data distributes this file under the Artistic License 2.0. The data originates from ISBN International (https://www.isbn-international.org/range_file_generation).
- **Why pinned:** hyphenation output depends on the range data, so byte-compatibility with biber 2.22 requires this exact snapshot. It is compiled into the binary at build time, so no network access is needed to build or run bibers.
- **Override:** like biber, bibers honours the `ISBN_RANGE_MESSAGE` environment variable to load a newer range file at run time.
- **Updating:** when following a newer biber release, replace the file with the one that release bundles and update this README.
