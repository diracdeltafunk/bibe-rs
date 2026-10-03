#!/usr/bin/env perl
# Differential vectors for crates/perl-compat (truth, case, unicode::ucd_num,
# roman, graphemes). Deterministic (srand), so the committed output
# crates/perl-compat/tests/data/scalar_vectors.json can be re-checked:
#
#   vendor/perl/bin/perl tools/oracle/probes/perl-compat/scalar_vectors.pl > \
#       crates/perl-compat/tests/data/scalar_vectors.json
use v5.24;
use strict;
use warnings;
no warnings qw(numeric uninitialized surrogate nonchar non_unicode utf8);
use JSON::PP;
use Scalar::Util qw(looks_like_number);
use Unicode::UCD qw(num);
use Unicode::GCString;
use Unicode::Normalize qw(NFKD);
use Text::Roman qw(isroman roman2int);

srand(20261003);
sub pick { $_[int rand @_] }

# ---------------------------------------------------------------- numbers
my @num_atoms = ('0', '1', '9', '12', '007', '-', '+', '.', 'e', 'E', ' ', "\t", "\n", "\x0B",
  'x', 'Inf', 'inf', 'Infinity', 'NaN', 'nan', 'nanq', '(12)', '1.#INF', '1.#IND', '_', 'a',
  "\x{663}", "\x{FF11}", '18446744073709551615', '18446744073709551616', '9223372036854775807',
  '9223372036854775808', '1e400', '0 but true', '#', 'q', 's', 'i', 'n');
my @nums = ('', ' ', '0', '00', '0.0', '1', '-1', '+1', ' 12 ', "12\n", '1.', '.5', '.', '1e5',
  '1e', '1e+', '-1.5e-3', '0x1A', '1_000', 'Inf', '-Inf', 'nan', 'NaN(123)', 'nan(0x1F)', 'infinity',
  'infin', '1.#INF', '-1.#IND', '0 but true', '0 but true ', '12abc', 'abc', "\x{663}",
  '9223372036854775807', '9223372036854775808', '-9223372036854775808', '-9223372036854775809',
  '18446744073709551615', '18446744073709551616', '1e15', '1e16', '123456789012345678',
  '0.1', '3.14159265358979323846', '-0', '-0.0', ' +.5', '1.5.2', '--1', '1 ', ' 1', "1\r\n",
  'snan', 'qnan', 'nans', 'Inf ', ' Infinity', 'inFINITY', 'nan(', 'nan()', 'nan(1x)', '1.#QNAN');
for (1 .. 400) {
  push @nums, join('', map { pick(@num_atoms) } 1 .. 1 + int rand 4);
}
my @numv;
for my $s (@nums) {
  my $n = $s + 0;
  push @numv, { s => "$s", lln => (looks_like_number($s) ? JSON::PP::true : JSON::PP::false),
    num => "$n", abs => '' . abs($s), sub1 => '' . ($s - 1), add2 => '' . ($s + 0.5) };
}
my @cmpv;
for (1 .. 150) {
  my ($a, $b) = (pick(@nums), pick(@nums));
  my $r = $a <=> $b;
  push @cmpv, { a => "$a", b => "$b", cmp => (defined $r ? $r : undef), sub => '' . ($a - $b) };
}

# ---------------------------------------------------------------- ucd num
my @digits = map { chr } (0x30 .. 0x39, 0x660 .. 0x669, 0x6F0 .. 0x6F9, 0x966 .. 0x96F,
  0xFF10 .. 0xFF19, 0x1D7CE .. 0x1D7D7, 0xBD, 0x2155, 0x216B, 0xF33, 0x4E07, 0x5146);
my @ucd = ('', '0', '00', '0012', '1x', '-1', ' 1', "1\n", "\x{663}\x{664}", "1\x{663}",
  "\x{FF11}\x{FF12}", "\x{FF11}2", "\x{216B}\x{216B}", "\x{BD}", "\x{BD}1", '18446744073709551616',
  '99999999999999999999999');
for (1 .. 300) {
  push @ucd, join('', map { pick(@digits) } 1 .. 1 + int rand 5);
}
for (1 .. 100) {
  my @blk = (0x30, 0x660, 0x966, 0xFF10);
  my $z = pick(@blk);
  push @ucd, join('', map { chr($z + int rand 10) } 1 .. 1 + int rand 8);
}
my @ucdv = map { my $v = num($_); { s => $_, num => (defined $v ? "$v" : undef) } } @ucd;

# ---------------------------------------------------------------- roman
my @rom_atoms = (qw(I V X L C D M i v x l c d m), ' ', "\n", "\x{131}", "\x{2160}", "\x{216B}",
  "\x{2170}", 'Q', '1');
my @rom = ('', 'XXiv', 'MCMXCIV', 'IIX', 'IM', 'X I', ' ', "XIV\n", "XIV\n\n", 'IIII', 'VV', 'IVI',
  "\x{2165}\x{2160}", "\x{217B}", 'MMMM', 'CMC', 'xc', 'ivx');
for (1 .. 400) { push @rom, join('', map { pick(@rom_atoms) } 1 .. 1 + int rand 6) }
my @romv;
for my $s (@rom) {
  my $k = NFKD($s);
  my $v = roman2int($k);
  push @romv, { s => $s, nfkd => $k, isroman => (isroman($k) ? JSON::PP::true : JSON::PP::false),
    roman2int => $v };
}

# ---------------------------------------------------------------- strings
my @pool = map { chr hex } qw(61 62 41 5A 20 D A 9 301 308 327 200D 200C FE0F 1F1E6 1F1E9 1F1EA
  1F600 1F3FB 1F468 1F469 2764 1100 1161 11A8 AC00 AC01 915 937 94D 93F 940 903 E01 E33 E40 600
  110BD 11A3A 1F 7F AD 34F 2028 DF 130 131 3A3 3C2 1C4 1C5 1C6 FB00 1E9E 1F80 345 69 307 49 E9 C9
  2160 FF21 10400 1E900 5D0 5B4 1B05 1B44 A8F2 11F02 11F41 D4D 0D15 1031 102D 1A60);
my @strs;
for (1 .. 1500) { push @strs, join('', map { pick(@pool) } 1 .. 1 + int rand 8) }
push @strs, ('', "\r\n", " \x{301}", "\x{915}\x{94D}\x{915}\x{915}", "a\x{94D}\x{915}",
  "\x{1F1E6}\x{1F1E9}\x{1F1EA}", "\x{1F468}\x{200D}\x{1F469}\x{200D}\x{1F467}");
my @strv;
for my $s (@strs) {
  my $gc = Unicode::GCString->new($s);
  my @gcl = map { length $_->as_string } $gc->as_array;
  my @xl = map { length } split /\b{gcb}/, $s;
  my @subs;
  for my $o (-3, -1, 0, 1, 2) {
    for my $l (undef, -2, 0, 1, 3) {
      my $r = defined $l ? Unicode::GCString->new($s)->substr($o, $l) : Unicode::GCString->new($s)->substr($o);
      push @subs, $r->as_string;
    }
  }
  push @strv, { s => $s, gcstring => \@gcl, gclength => $gc->length, perlx => \@xl, substr => \@subs,
    lc => lc $s, uc => uc $s, fc => fc $s, ucfirst => ucfirst $s, lcfirst => lcfirst $s };
}

my $json = JSON::PP->new->utf8->canonical;
print $json->encode({ numbers => \@numv, numcmp => \@cmpv, ucd_num => \@ucdv, roman => \@romv,
  strings => \@strv });
