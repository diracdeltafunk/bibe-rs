#!/usr/bin/env perl
# Exhaustive scan: for which code points C does btparse treat a name token
# starting with C as lower-case (i.e. part of the "von" particle)?
# Probe: split_name("Jean <C>x Fontaine") and check whether "<C>x" lands in von.
# Output: ranges "start-end" (hex) of code points classified lower-case, plus a
# comparison with Perl's General_Category=Ll restricted to each Unicode age.
# Usage: vendor/perl/bin/perl -Itools/oracle/probes/btparse scan_lower.pl > lower_ranges.txt
use strict; use warnings;
use FindBin; use lib $FindBin::Bin;
use BtProbe;
use Unicode::Normalize qw(NFC);

my @lower;
for my $cp (0x80 .. 0x10FFFF) {
  next if $cp >= 0xD800 && $cp <= 0xDFFF;
  my $c = chr($cp);
  next if NFC($c) ne $c;   # biber passes NFC strings to btparse
  my $r = BtProbe::split_name("Jean ${c}x Fontaine");
  my $v = $r->{von};
  push @lower, $cp if @$v && defined $v->[0];
}
BtProbe::restore_stderr();
# ranges
my @ranges;
for my $cp (@lower) {
  if (@ranges && $ranges[-1][1] == $cp - 1) { $ranges[-1][1] = $cp } else { push @ranges, [$cp, $cp] }
}
printf "%04X-%04X\n", @$_ for @ranges;
# compare with Ll by age
my %got = map { $_ => 1 } @lower;
for my $age (qw(5.0 5.1 5.2 6.0 6.1 6.2 6.3 7.0 8.0 16.0)) {
  my ($missing, $extra) = (0, 0);
  for my $cp (0x80 .. 0x10FFFF) {
    next if $cp >= 0xD800 && $cp <= 0xDFFF;
    my $c = chr($cp);
    next if NFC($c) ne $c;
    my $ll = ($c =~ /\p{Ll}/ && $c =~ /\p{Present_In: $age}/) ? 1 : 0;
    $missing++ if $ll && !$got{$cp};
    $extra++ if !$ll && $got{$cp};
  }
  print STDERR "age $age: btparse-lower not Ll=$extra, Ll not btparse-lower=$missing\n";
}
