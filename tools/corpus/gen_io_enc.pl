#!/usr/bin/env perl
# Generate the encoding cases of tests/corpus/io/ (M4 io package, DC-034):
# multi-byte --output-encoding and --input-encoding over one .bib with
# Japanese, Chinese, Korean, Cyrillic and Latin text, long enough that
# multi-byte characters straddle PerlIO's 1024-byte chunks (BB-152).
# Goldens: cargo xtask golden io/enc-
#
# Usage: perl tools/corpus/gen_io_enc.pl
use strict;
use warnings;
use utf8;
use Encode qw(encode);
use FindBin;
use File::Path qw(make_path);
use File::Copy qw(copy);

my $root = "$FindBin::Bin/../..";
my $dir  = "$root/tests/corpus/io";
my $bcf  = "$root/tests/corpus/ported/encoding-4/encoding6.bcf";

my $jp = '日本語の文献目録。カタカナ、ひらがな、漢字。半角ｶﾀｶﾅ。';
my $zh = '中文参考文献，简体字与繁體字。';
my $ko = '한국어 참고 문헌 목록.';
my $abstract = join(' ', map { "$_ $jp $zh $ko Ünïcödé — “quotes” №$_ αβγ." } 1 .. 30);

# Entries: plain ASCII, Japanese only, all scripts (some characters are not
# encodable in most targets: the entry is then recoded into LaTeX macros
# with a WARN), and one with a very long abstract (chunk boundaries).
my $bib = <<"EOF";
% UTF-8
\@BOOK{ascii,
  AUTHOR = {Edward Encalcer},
  YEAR = {1999},
  TITLE = {A plain title},
  PUBLISHER = {A press}
}
\@BOOK{nihon,
  AUTHOR = {山田, 太郎 and 鈴木, 花子},
  YEAR = {2001},
  TITLE = {$jp},
  PUBLISHER = {東京出版}
}
\@ARTICLE{mixed,
  AUTHOR = {Müller, Jürgen and Иванов, Иван and 王, 小明},
  YEAR = {2010},
  TITLE = {Ŝome ŧitle with ﬁ ligature, € and ½},
  JOURNALTITLE = {$zh $ko},
  ABSTRACT = {$abstract}
}
\@MISC{long,
  AUTHOR = {김, 철수},
  YEAR = {2020},
  TITLE = {$ko},
  ABSTRACT = {@{[ join(' ', ($jp) x 60) ]}}
}
EOF

sub writecase {
  my ($name, $desc, $args, $bibbytes) = @_;
  my $d = "$dir/$name";
  make_path($d);
  copy($bcf, "$d/encoding6.bcf") or die;
  open my $fh, '>:raw', "$d/encoding6.bib" or die;
  print $fh $bibbytes;
  close $fh;
  my $a = join(', ', map { qq("$_") } '--noconf', @$args, 'encoding6.bcf');
  open my $c, '>:utf8', "$d/case.toml" or die;
  print $c <<"EOF";
description = "$desc"
source = "tools/corpus/gen_io_enc.pl (bibers M4 io package, DC-034); .bcf from t/tdata/encoding6.bcf"
args = [$a]
outputs = ["encoding6.bbl"]
tags = ["suite:io", "feature:encoding"]
EOF
  close $c;
}

my $utf8 = encode('UTF-8', $bib);
for my $e (qw(shiftjis cp932 euc-jp iso-2022-jp 7bit-jis euc-kr cp949 iso-2022-kr
              euc-cn cp936 hz big5-eten big5-hkscs cp950 UTF-16 UTF-16LE UTF-16BE
              UTF-32 UTF-32LE UCS-2BE koi8-r cp1251 utf-8-strict MacJapanese johab)) {
  (my $n = lc $e) =~ s/[^a-z0-9]+/-/g;
  writecase("enc-out-$n", "--output-encoding=$e (multi-byte output, unencodable entries recoded)",
            ["--output-encoding=$e"], $utf8);
}
# Aliases as users spell them.
for my $e (qw(sjis Shift_JIS GBK GB2312 Big5 UTF16 ucs2 EUC-JP)) {
  (my $n = lc $e) =~ s/[^a-z0-9]+/-/g;
  writecase("enc-out-alias-$n", "--output-encoding=$e (alias resolution)",
            ["--output-encoding=$e"], $utf8);
}
# --output-safechars with a multi-byte encoding.
writecase("enc-out-shiftjis-safechars", "--output-encoding=shiftjis --output-safechars",
          ['--output-encoding=shiftjis', '--output-safechars'], $utf8);

# Input encodings: the same data, encoded (characters the encoding lacks are
# dropped from the input text first).
for my $e (qw(shiftjis cp932 euc-jp euc-kr cp949 euc-cn cp936 big5-eten UTF-16 UTF-16LE UTF-32BE)) {
  my $text = $bib;
  my $enc = Encode::find_encoding($e);
  $text = join('', grep { length(eval { $enc->encode($_, Encode::FB_CROAK() | Encode::LEAVE_SRC()) } // '') } split //, $text);
  my $bytes = $enc->encode($text);
  (my $n = lc $e) =~ s/[^a-z0-9]+/-/g;
  writecase("enc-in-$n", "--input-encoding=$e (multi-byte data source), UTF-8 output",
            ["--input-encoding=$e"], $bytes);
}
# A UTF-16 file with a BOM read as UTF-16LE (BOM kept as U+FEFF, then stripped by biber).
writecase("enc-in-utf-16le-bom", "--input-encoding=UTF-16LE on a file with a BOM",
          ['--input-encoding=UTF-16LE'], encode('UTF-16LE', "\x{FEFF}$bib"));
# Output and input both multi-byte.
writecase("enc-inout-sjis-eucjp", "--input-encoding=shiftjis --output-encoding=euc-jp",
          ['--input-encoding=shiftjis', '--output-encoding=euc-jp'],
          Encode::find_encoding('shiftjis')->encode(join('', grep {
            length(eval { encode('shiftjis', $_, Encode::FB_CROAK() | Encode::LEAVE_SRC()) } // '') } split //, $bib)));
# UTF-16 with a surrogate pair cut by PerlIO's 1024-byte read chunks: biber
# cannot read the file ("UTF-16:Partial character"; quirk utf16-chunk-surrogate).
# BOM (2 bytes) + 510 BMP characters put the high surrogate at byte 1022.
{
  my $pre = "% padding ";
  my $text = $pre . ('x' x (510 - length($pre) - 1)) . "\n\x{1F600}\n$bib";
  writecase("enc-in-utf-16-surrogate-split",
            "--input-encoding=UTF-16 with a surrogate pair across PerlIO's 1024-byte chunk boundary",
            ['--input-encoding=UTF-16'], encode('UTF-16', $text));
  # The read error lacks perl's " at FILE line N." (docs/compat.md DC-046).
  open my $c, '>>', "$dir/enc-in-utf-16-surrogate-split/case.toml" or die;
  print $c qq(allow = ["DC-046"]\n);
  close $c;
}
# The same character elsewhere is fine.
writecase("enc-in-utf-16-astral", "--input-encoding=UTF-16 with characters outside the BMP",
          ['--input-encoding=UTF-16'], encode('UTF-16', "% \x{1F600}\n$bib"));
# Tool mode (bibtex output) with multi-byte output encodings: its own
# "can this entry be encoded" test is `encode(...) =~ /\?/` (BB-048).
for my $e (qw(shiftjis euc-kr cp936 big5-eten UTF-16 UTF-16LE iso-2022-jp)) {
  (my $n = lc $e) =~ s/[^a-z0-9]+/-/g;
  my $d = "$dir/tool-out-$n";
  make_path($d);
  open my $fh, '>:raw', "$d/data.bib" or die;
  print $fh $utf8, encode('UTF-8', "\@MISC{q,\n  TITLE = {Why? 日本}\n}\n");
  close $fh;
  open my $c, '>:utf8', "$d/case.toml" or die;
  print $c <<"EOF";
description = "tool mode, --output-encoding=$e (bibtex output; BB-048 '?' test)"
source = "tools/corpus/gen_io_enc.pl (bibers M4 io package, DC-034)"
args = ["--noconf", "--tool", "--output-encoding=$e", "data.bib"]
outputs = ["data_bibertool.bib"]
log = "data.bib.blg"
tags = ["suite:io", "feature:encoding", "feature:tool"]
EOF
  close $c;
}
print STDERR "done\n";
