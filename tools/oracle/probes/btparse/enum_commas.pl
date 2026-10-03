#!/usr/bin/env perl
# Enumerate all token/comma patterns up to length N (tokens are distinct
# upper-case letters, separated by spaces; commas attached without spaces) and
# print how btparse distributes the tokens over first/von/last/jr.
use strict; use warnings;
use FindBin; use lib $FindBin::Bin;
use BtProbe;
my $N = shift // 7;
my @pats = ('');
for my $len (1 .. $N) {
  my @next;
  for my $p (@pats) { push @next, $p . 'T', $p . ',' }
  @pats = @next;
  for my $p (@pats) {
    next unless $p =~ /,/;
    my $i = 0;
    my $s = '';
    for my $ch (split //, $p) {
      if ($ch eq 'T') { $s .= ' ' if length($s) && substr($s, -1) ne ','; $s .= chr(ord('A') + $i++) }
      else { $s .= ',' }
    }
    my $r = BtProbe::isolated(sub { BtProbe::split_name($s) });
    if ($r->{crash}) { print "$p\t$s\tCRASH\n"; next }
    my %o;
    for my $k (qw(first von last jr)) { $o{$k} = join('', map { $_ // '' } @{$r->{$k}}) }
    my $w = join(';', map { /too many/ ? 'MANY' : /at end/ ? 'END' : $_ } @{$r->{diags}});
    print "$p\t$s\tL=$o{last} J=$o{jr} F=$o{first} V=$o{von}\t$w\n";
  }
}
BtProbe::restore_stderr();
