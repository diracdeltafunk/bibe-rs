#!/usr/bin/env perl
# Interactive black-box probe of Text::BibTeX as biber uses it.
#   bt.pl parse [biber|tb|none]  < file.bib   # parse one file, dump entries + diagnostics
#   bt.pl list DELIM             < lines      # split_list each line
#   bt.pl name                   < lines      # split_name + biber's 4 formats, each line
#   bt.pl rawname                < lines      # same, without biber's pre-sanitising
# Run: vendor/perl/bin/perl -Itools/oracle/probes/btparse tools/oracle/probes/btparse/bt.pl ...
use strict;
use warnings;
use FindBin;
use lib $FindBin::Bin;
use BtProbe;
use Encode qw(decode);

my $mode = shift // 'parse';
my $J = JSON::PP->new->utf8->canonical->allow_nonref;
binmode STDOUT; $| = 1;
local $/;
my $in = decode('UTF-8', scalar(<STDIN>) // '');

if ($mode eq 'cases') {
  # stdin: cases separated by lines "### ID description"; each parsed as its own file
  my @cases = split /^### /m, $in;
  shift @cases;
  for my $c (@cases) {
    my ($hdr, $body) = split /\n/, $c, 2;
    $body //= '';
    $body =~ s/\n\z// unless $body =~ /\n\n\z/;
    my $r = BtProbe::isolated(sub { BtProbe::reset_macros('biber'); BtProbe::parse_text($body) });
    print "### $hdr\n";
    if ($r->{crash}) { print "  CRASH signal $r->{crash}\n"; print "    ! $_\n" for $r->{diags}->@*; next }
    for my $e ($r->{entries}->@*) {
      my $d = delete $e->{diags};
      print "  ", $J->encode($e), "\n";
      print "    ! $_\n" for @$d;
    }
    print "    ! (tail) $_\n" for $r->{tail}->@*;
  }
}
elsif ($mode eq 'parse') {
  BtProbe::reset_macros(shift // 'biber');
  my $r = BtProbe::parse_text($in);
  for my $e ($r->{entries}->@*) { print $J->encode($e), "\n" }
  print "tail: ", $J->encode($r->{tail}), "\n" if $r->{tail}->@*;
}
else {
  for my $line (split /\n/, $in) {
    my $r;
    if ($mode eq q{list}) { my $d = $ARGV[0] // q{and}; $r = BtProbe::isolated(sub { BtProbe::split_list_biber($line, $d) }) }
    elsif ($mode eq q{name}) { $r = BtProbe::isolated(sub { BtProbe::split_name(BtProbe::biber_sanitise($line)) }) }
    elsif ($mode eq q{rawname}) { $r = BtProbe::isolated(sub { BtProbe::split_name($line) }) }
    elsif ($mode eq q{show}) {
      # human-readable name dump
      $r = BtProbe::isolated(sub { BtProbe::split_name(BtProbe::biber_sanitise($line)) });
      binmode STDOUT, ':utf8';
      if ($r->{crash}) { print "$line CRASH($r->{crash}) @{$r->{diags}}\n"; next }
      my %p;
      for my $k (qw(first von last jr)) {
        $p{$k} = join('|', map { !defined $_ ? q{-} : utf8::is_utf8($_) ? $_ : Encode::decode(q{UTF-8}, $_) } @{$r->{$k}});
      }
      printf "%-36s F=%s V=%s L=%s J=%s || f=[%s] v=[%s] l=[%s] j=[%s] %s\n", $line,
        @p{qw(first von last jr)}, (map { $r->{fmt}{$_} } qw(f v l j)), join(';', @{$r->{diags}});
      next;
    }
    print $J->encode($line), " => ", $J->encode($r), "\n";
  }
}
BtProbe::restore_stderr();
