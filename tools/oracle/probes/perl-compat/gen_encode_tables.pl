#!/usr/bin/env perl
# Generate crates/perl-compat/data/encode_tables.bin: the compiled conversion
# tables of every table-driven Perl `Encode` encoding (Encode::XS objects:
# Encode core, Encode::Byte, ::CN, ::JP, ::KR, ::TW, ::EBCDIC, ::Symbol), as
# bundled with biber 2.22 (Encode 3.21, oracle O3).
#
# The input is the C that enc2xs generated from Encode's .ucm files when O3's
# perl was built (vendor/perl-build/perl-5.42.0/cpan/Encode/*/*_t.c and
# def_t.c): arrays of `encpage_t` {seq, next, min, max, dlen, slen} and one
# `encode_t` per encoding. These are exactly the tables Encode's engine
# (encengine.c `do_encode`) walks at run time, so perl-compat's port of the
# engine reproduces Encode byte for byte, including partial-character and
# fallback behaviour.
#
# Output format (all integers little-endian):
#   "PCEN" u32 version(1)
#   u32 nbytes, bytes             -- all enctable[] arrays concatenated
#   u32 npages; per page: u16 nentries; per entry:
#       u8 min, u8 max, u8 dlen, u8 slen, u32 next_page, u32 seq (0xFFFFFFFF = none)
#   u32 nenc; per encoding:
#       u8 len, name, u8 len, rep, u8 min_el, u8 max_el, u32 to_utf8_page, u32 from_utf8_page
#
# Usage: vendor/perl/bin/perl tools/oracle/probes/perl-compat/gen_encode_tables.pl
use strict;
use warnings;
use FindBin;

my $root = "$FindBin::Bin/../../../..";
my $encdir = "$root/vendor/perl-build/perl-5.42.0/cpan/Encode";
my @files = (sort(glob("$encdir/*/*_t.c")), "$encdir/def_t.c");
die "no enc2xs output under $encdir (build O3 first)\n" unless -f "$encdir/def_t.c";

my $bytes = '';
my @pages;      # [ [min,max,dlen,slen,nextname,seq], ... ] with global names
my %pageidx;    # "file:name" -> index
my @encs;

for my $f (@files) {
  open my $fh, '<', $f or die "$f: $!";
  local $/;
  my $c = <$fh>;
  close $fh;
  my $tag = $f;
  # Only the C (not C++) variants of the definitions.
  $c =~ s/#ifdef __cplusplus\n.*?\n#else\n(.*?)\n#endif/$1/gs;
  my $base = length $bytes;
  if ($c =~ /static const U8 enctable\[(\d+)\] = \{(.*?)\};/s) {
    my ($n, $body) = ($1, $2);
    my @b = ($body =~ /(\d+)/g);
    die "$f: enctable size" unless @b == $n;
    $bytes .= pack('C*', @b);
  }
  my %rep;
  while ($c =~ /static const U8 (\w+)_rep_character\[\] = "((?:\\x[0-9a-fA-F]{2})*)";/g) {
    my ($n, $r) = ($1, $2);
    $r =~ s/\\x([0-9a-fA-F]{2})/chr(hex $1)/ge;
    $rep{$n} = $r;
  }
  my %names;
  while ($c =~ /static const char (\w+)_enc_name\[\] = "([^"]*)";/g) {
    $names{$1} = $2;
  }
  my @local;
  while ($c =~ /static const encpage_t (\w+)\[(\d+)\] = \{\n(.*?)\n\};/gs) {
    my ($name, $n, $body) = ($1, $2, $3);
    my @e;
    for my $l (split /\n/, $body) {
      $l =~ /^\{(0|enctable \+ (\d+)),(\w+),0x([0-9a-f]+),0x([0-9a-f]+),(\d+),(\d+)\},?$/
        or die "$f: bad entry $l";
      push @e, [hex $4, hex $5, $6, $7, "$tag:$3", defined $2 ? $base + $2 : 0xFFFFFFFF];
    }
    die "$f: $name count" unless @e == $n;
    $pageidx{"$tag:$name"} = scalar @pages;
    push @pages, \@e;
  }
  while ($c =~ /const encode_t (\w+)_encoding = \s*\{(\w+),(\w+),(\w+)_rep_character,(\d+),(\d+),(\d+),\{(\w+)_enc_name,/g) {
    my ($id, $t, $fr, $repn, $replen, $min, $max, $nm) = ($1, $2, $3, $4, $5, $6, $7, $8);
    die "$f: rep $repn" unless defined $rep{$repn} && length $rep{$repn} >= $replen;
    # Only the first replen bytes are used (gb12345-raw declares "\x22\x3F", length 1).
    push @encs, [$names{$nm} // die("$f: name $nm"), substr($rep{$repn}, 0, $replen), $min, $max, "$tag:$t", "$tag:$fr"];
  }
}

my $out = "PCEN" . pack('V', 1);
$out .= pack('V', length $bytes) . $bytes;
$out .= pack('V', scalar @pages);
for my $p (@pages) {
  $out .= pack('v', scalar @$p);
  for my $e (@$p) {
    my $next = $pageidx{$e->[4]} // die "unknown page $e->[4]";
    $out .= pack('CCCCVV', $e->[0], $e->[1], $e->[2], $e->[3], $next, $e->[5]);
  }
}
$out .= pack('V', scalar @encs);
for my $e (sort { $a->[0] cmp $b->[0] } @encs) {
  my ($name, $rep, $min, $max, $t, $fr) = @$e;
  $out .= pack('C/a* C/a* CC VV', $name, $rep, $min, $max,
               $pageidx{$t} // die("page $t"), $pageidx{$fr} // die("page $fr"));
}
my $dst = "$root/crates/perl-compat/data/encode_tables.bin";
open my $o, '>:raw', $dst or die "$dst: $!";
print $o $out;
close $o;
printf STDERR "%d encodings, %d pages, %d table bytes, %d bytes\n",
  scalar @encs, scalar @pages, length $bytes, length $out;
