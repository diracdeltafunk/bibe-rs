#!/usr/bin/env perl
# Oracle for btparse differential tests.
# Reads JSON-lines requests on stdin, writes JSON-lines vectors on stdout:
#   {"op":"parse","text":T}            -> {"op":"parse","text":NFD(T),"out":{...}}
#   {"op":"list","s":S,"d":D}          -> {"op":"list","s":NFC(S),"d":D,"out":{...}}
#   {"op":"name","s":S}                -> {"op":"name","s":NFC(S),"out":{...}}
# Every request runs in a forked child (libbtparse can crash); crashes are
# recorded as {"crash":SIG,"diags":[...]}.
# Parsing uses biber's macro state (Text::BibTeX defaults + months 1..12),
# biber's open options (binmode utf-8, NFD) and its loop
# `while (my $e = Text::BibTeX::Entry->new($bib))`.
# Run: vendor/perl/bin/perl -Itools/oracle/probes/btparse vectors.pl < req.jsonl > vec.jsonl
use strict; use warnings;
use FindBin; use lib $FindBin::Bin;
use BtProbe;
use Unicode::Normalize qw(NFD NFC);
use Encode ();

my $J = JSON::PP->new->utf8->canonical;
$| = 1;
binmode STDIN; binmode STDOUT;

sub dec { my $x = shift; return undef unless defined $x; utf8::is_utf8($x) ? $x : Encode::decode('UTF-8', $x, Encode::FB_PERLQQ) }

while (my $line = <STDIN>) {
  next unless $line =~ /\S/;
  my $req = $J->decode($line);
  my $op = $req->{op};
  my %v = (op => $op);
  my $out;
  if ($op eq 'parse') {
    my $text = NFD($req->{text});
    $v{text} = $text;
    $out = BtProbe::isolated(sub {
      BtProbe::reset_macros('biber');
      my $r = BtProbe::parse_text($text);
      my @diags;
      my @ents;
      for my $e ($r->{entries}->@*) {
        push @diags, $e->{diags}->@*;
        my %c = (mt => $e->{metatype}, type => dec($e->{type}), ok => $e->{ok});
        $c{key} = dec($e->{key}) if exists $e->{key};
        $c{fields} = [ map { [dec($_->[0]), $_->[1]] } $e->{fields}->@* ] if $e->{fields};
        $c{value} = $e->{value} if exists $e->{value};
        push @ents, \%c;
      }
      push @diags, $r->{tail}->@*;
      return { entries => \@ents, diags => \@diags };
    });
  }
  elsif ($op eq 'list') {
    $v{s} = NFC($req->{s});
    $v{d} = $req->{d};
    $out = BtProbe::isolated(sub { BtProbe::split_list_biber($v{s}, $v{d}) });
  }
  elsif ($op eq 'name') {
    $v{s} = NFC($req->{sanitise} ? BtProbe::biber_sanitise($req->{s}) : $req->{s});
    $out = BtProbe::isolated(sub {
      my $r = BtProbe::split_name($v{s});
      for my $p (qw(first von last jr)) {
        $r->{$p} = [ grep { defined } map { dec($_) } $r->{$p}->@* ];
      }
      return $r;
    });
  }
  else { die "bad op $op" }
  $v{out} = $out;
  $v{id} = $req->{id} if exists $req->{id};
  print $J->encode(\%v), "\n";
}
BtProbe::restore_stderr();
