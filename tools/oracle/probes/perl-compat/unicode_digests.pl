#!/usr/bin/env perl
# Exhaustive per-code-point oracle for crates/perl-compat: prints a JSON object
# of SHA-256 digests, one per behaviour, over every code point except
# surrogates. The Rust test `tests/unicode_exhaustive.rs` computes the same
# serialisation and compares with the committed copy
# (crates/perl-compat/tests/data/unicode_digests.json).
#
#   vendor/perl/bin/perl -Ivendor/perl-build/biber-src/blib/lib \
#       tools/oracle/probes/perl-compat/unicode_digests.pl [only-key ...]
#
# Serialisation (must match the Rust side exactly):
#   mappings (lc uc fc tc nfd nfc nfkd nfkc): "%X:%X %X ...\n" for each code
#     point whose result differs from the input;
#   props (prop:<syntax>): "%X\n" for each member;
#   num: "%X:<perl stringification>\n" for each defined num(chr);
#   gcstring / perlx: "%X:<n1>,<n2>,...\n" (cluster lengths in characters) of
#     the probe string context(c) for every code point.
use v5.24;
use strict;
use warnings;
no warnings qw(surrogate nonchar non_unicode utf8);
use Digest::SHA;
use Unicode::Normalize qw(NFD NFC NFKD NFKC);
use Unicode::UCD qw(num);
use Unicode::GCString;

my %only = map { $_ => 1 } @ARGV;
my $MAX = 0x10FFFF;

# The context string for grapheme tests; keep in sync with the Rust test.
my @PARTNERS = map { chr hex } qw(61 301 200D 1F1E6 1100 1161 11A8 AC00 AC01 20 D A 94D 915 600 1F600 1F3FB 903 E33 1F1E6 FE0F);
sub context {
  my $c = shift;
  return join('', map { ($c, $_) } @PARTNERS) . $c;
}

my %maps = (
  lc => sub { lc $_[0] }, uc => sub { uc $_[0] }, fc => sub { fc $_[0] },
  tc => sub { ucfirst $_[0] },
  nfd => \&NFD, nfc => \&NFC, nfkd => \&NFKD, nfkc => \&NFKC,
);
my @props = qw(\s \d \w \h \v \p{L} \p{Lu} \p{Ll} \p{Lt} \p{Lm} \p{Lo} \p{M} \p{Mn} \p{Mc}
  \p{Me} \p{N} \p{Nd} \p{Nl} \p{No} \p{P} \p{Pc} \p{Pd} \p{Ps} \p{Pe} \p{Pi} \p{Pf} \p{Po}
  \p{S} \p{Sm} \p{Sc} \p{Sk} \p{So} \p{Z} \p{Zs} \p{C} \p{Cc} \p{Cf} \p{Cn} \p{Dash}
  \p{Dia} \p{Alpha} \p{Upper} \p{Lower} \p{ASCII_Hex_Digit});

my %out;
sub want { !%only || $only{$_[0]} }

for my $k (sort keys %maps) {
  next unless want($k);
  my $f = $maps{$k};
  my $d = Digest::SHA->new(256);
  for my $cp (0 .. $MAX) {
    next if $cp >= 0xD800 && $cp <= 0xDFFF;
    my $c = chr $cp;
    my $m = $f->($c);
    next if $m eq $c;
    $d->add(sprintf("%X:%s\n", $cp, join(' ', map { sprintf '%X', ord } split //, $m)));
  }
  $out{$k} = $d->hexdigest;
}
for my $p (@props) {
  my $k = "prop:$p";
  next unless want($k) || want('props');
  my $re = qr/\A$p\z/;
  my $d = Digest::SHA->new(256);
  for my $cp (0 .. $MAX) {
    next if $cp >= 0xD800 && $cp <= 0xDFFF;
    $d->add(sprintf("%X\n", $cp)) if chr($cp) =~ $re;
  }
  $out{$k} = $d->hexdigest;
}
if (want('num')) {
  my $d = Digest::SHA->new(256);
  for my $cp (0 .. $MAX) {
    next if $cp >= 0xD800 && $cp <= 0xDFFF;
    my $v = num(chr $cp);
    $d->add(sprintf("%X:%s\n", $cp, $v)) if defined $v;
  }
  $out{num} = $d->hexdigest;
}
if (want('gcstring')) {
  my $d = Digest::SHA->new(256);
  for my $cp (0 .. $MAX) {
    next if $cp >= 0xD800 && $cp <= 0xDFFF;
    my $gc = Unicode::GCString->new(context(chr $cp));
    $d->add(sprintf("%X:%s\n", $cp, join(',', map { length $_->as_string } $gc->as_array)));
  }
  $out{gcstring} = $d->hexdigest;
}
if (want('perlx')) {
  my $d = Digest::SHA->new(256);
  for my $cp (0 .. $MAX) {
    next if $cp >= 0xD800 && $cp <= 0xDFFF;
    my @cl = split /\b{gcb}/, context(chr $cp);
    $d->add(sprintf("%X:%s\n", $cp, join(',', map { length } @cl)));
  }
  $out{perlx} = $d->hexdigest;
}

sub jkey { my $k = shift; $k =~ s/\\/\\\\/g; $k }
print "{\n", join(",\n", map { '  "' . jkey($_) . qq{": "$out{$_}"} } sort keys %out), "\n}\n";
