#!/usr/bin/env perl
# Real-world differential input: turn .bib files into oracle "parse" requests
# (after biber's preprocessing: UTF-8 decode, BOM strip, \R -> \n), plus one
# "list"/"name" request per author/editor-looking value is left to the Rust
# side. Files that are not valid UTF-8 are skipped.
#   files2req.pl FILE... > req.jsonl
use strict; use warnings;
use JSON::PP;
use Encode qw(decode);
my $J = JSON::PP->new->utf8->canonical;
for my $f (@ARGV) {
  open my $fh, '<:raw', $f or next;
  local $/;
  my $raw = <$fh>;
  my $s = eval { decode('UTF-8', $raw, Encode::FB_CROAK) };
  next unless defined $s;
  next unless $s =~ /\@/;
  $s =~ s/\A\x{feff}//;
  $s =~ s/\R/\n/g;
  print $J->encode({ id => $f, op => 'parse', text => $s }), "\n";
}
