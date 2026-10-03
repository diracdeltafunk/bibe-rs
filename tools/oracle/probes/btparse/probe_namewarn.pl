#!/usr/bin/env perl
# Where do split_name diagnostics come from (Name->new vs format)?
use strict; use warnings;
use FindBin; use lib $FindBin::Bin;
use BtProbe;
use Text::BibTeX::Name;
binmode STDOUT, ':utf8';
for my $s ('Doe {John', 'Doe, John,', 'A, B, C, D', 'Doe} John') {
  my $r = BtProbe::isolated(sub {
    BtProbe::take_stderr();
    my $n = Text::BibTeX::Name->new({binmode => 'utf-8', normalization => 'NFD'}, $s);
    my $d1 = BtProbe::take_stderr();
    my @l = $n->part('last');
    my $d2 = BtProbe::take_stderr();
    my $f = $n->format(BtProbe::_fmt('l'));
    my $d3 = BtProbe::take_stderr();
    return { new => $d1, part => $d2, format => $d3 };
  });
  print "$s => ", JSON::PP->new->canonical->encode($r), "\n";
}
BtProbe::restore_stderr();
