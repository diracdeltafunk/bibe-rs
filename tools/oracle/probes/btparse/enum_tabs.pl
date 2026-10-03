#!/usr/bin/env perl
# Which tabs inside a string does btparse turn into spaces? Enumerate all
# brace-balanced sequences over {tab, newline, '{', '}', 'x'} and show the
# raw (uncollapsed) @string value; T marks a kept tab, _ a space.
use strict; use warnings;
use FindBin; use lib $FindBin::Bin;
use BtProbe;
my $N = shift // 6;
my %map = (T => "\t", N => "\n", O => '{', C => '}', x => 'x');
my @pats = ('');
for my $len (1 .. $N) {
  @pats = map { my $p = $_; map { $p . $_ } qw(T N O C x) } @pats;
  for my $p (@pats) {
    next unless $p =~ /T/ && $p =~ /N/;
    my $d = 0; my $ok = 1;
    for (split //, $p) { $d++ if $_ eq 'O'; $d-- if $_ eq 'C'; $ok = 0 if $d < 0 }
    next unless $ok && $d == 0;
    my $s = join '', map { $map{$_} } split //, $p;
    for my $q ('{', '"') {
      my $body = $q eq '{' ? "{a${s}b}" : "\"a${s}b\"";
      my $r = BtProbe::parse_text("\@string{m = $body}");
      my $v = $r->{entries}[0]{fields}[0][1];
      $v =~ s/\t/T/g; $v =~ s/ /_/g;
      print "$q $p\t$v\n";
    }
  }
}
BtProbe::restore_stderr();
