#!/usr/bin/env perl
# Replay one traced configuration in a *fresh* in-process biber (O3) and print
# the .bbl (with sortdebug lines when --sortdebug is given).
#
# The configuration (from tools/port-tests/out/cases.json, written by gen.py)
# is applied in the same way the .t file does it -- Biber->new, parse_ctrlfile,
# setoption, setblxoption, set_sortingtemplatename -- but without the earlier
# prepare() calls of the test. Comparing the result with the CLI golden tells
# whether a difference between a .t expectation and the golden comes from the
# CLI translation (replay == .t) or from state the test carried over between
# prepare() calls (replay == golden).
#
# Usage (run via tools/port-tests/replay.sh): replay.pl <case> <out.bbl> [--sortdebug]
use strict;
use warnings;
use JSON::PP;
use Biber;
use Biber::Output::bbl;
use Log::Log4perl;

my ($case, $outfile, @flags) = @ARGV;
my $root = $ENV{PORT_ROOT} or die "PORT_ROOT not set";
my $cases = do {
  local $/;
  open my $fh, '<', "$root/tools/port-tests/out/cases.json" or die $!;
  decode_json(<$fh>);
};
my $c = $cases->{$case} or die "no case $case";
my $cfg = $c->{config};

my $l4pconf = q|
    log4perl.category.main                             = ERROR, Screen
    log4perl.category.screen                           = ERROR, Screen
    log4perl.appender.Screen                           = Log::Log4perl::Appender::Screen
    log4perl.appender.Screen.utf8                      = 1
    log4perl.appender.Screen.Threshold                 = ERROR
    log4perl.appender.Screen.stderr                    = 1
    log4perl.appender.Screen.layout                    = Log::Log4perl::Layout::SimpleLayout
|;
Log::Log4perl->init(\$l4pconf);
chdir("$root/vendor/biber/t/tdata") or die;

sub unser {
  my $v = shift;
  if (ref($v) eq 'HASH') {
    return qr/$v->{__regex}/ if exists $v->{__regex};
    return { map { $_ => unser($v->{$_}) } keys %$v };
  }
  if (ref($v) eq 'ARRAY') { return [ map { unser($_) } @$v ] }
  return $v;
}

my $biber = Biber->new(%{$cfg->{new_args}});
Biber::Config->setoption($_, unser($cfg->{opts_pre}{$_})) foreach keys %{$cfg->{opts_pre}};
# annotations.t: the marker regex is cached at load time
if (my $am = $cfg->{opts_pre}{annotation_marker}) {
  $Biber::Config::CONFIG_META_MARKERS{annotation} = quotemeta($am);
}
$biber->parse_ctrlfile($cfg->{bcf});
# State the tests set outside traced subs (see gen.py OVERRIDES)
foreach my $a (@{$c->{args}}) {
  if ($a =~ /^--input-encoding=(.+)$/) {
    my $enc = $1;
    $_->{encoding} = $enc foreach $biber->sections->get_section(0)->get_datasources->@*;
  }
  if ($a eq '--decodecharsset=full') { Biber::LaTeX::Recode->init_sets('full', 'full') }
}
if ($c->{tfile} eq 'crossrefs') { $Biber::Constants::DATAFIELD_SETS{nobtitle} = ['booktitle'] }
$biber->set_output_obj(Biber::Output::bbl->new());
# keep the order of opts as recorded
foreach my $k (keys %{$cfg->{opts}}) {
  Biber::Config->setoption($k, unser($cfg->{opts}{$k}));
}
foreach my $b (@{$cfg->{blx}}) {
  my ($o, $v, $s, $sv) = @$b;
  Biber::Config->setblxoption(undef, $o, unser($v), $s, $sv);
}
foreach my $l (keys %{$cfg->{tmplname}}) {
  $biber->datalists->get_list($l)->set_sortingtemplatename($cfg->{tmplname}{$l});
}
foreach my $m (@{$cfg->{mods}}) {
  my ($sub, $args) = @$m;
  if ($sub eq 'Biber::Section::set_allkeys') { $biber->sections->get_section(0)->set_allkeys(@$args) }
  elsif ($sub eq 'Biber::Section::del_citekeys') { $biber->sections->get_section(0)->del_citekeys }
  elsif ($sub eq 'Biber::Config::reset_keyorder') { Biber::Config->reset_keyorder(@$args) }
}
$biber->prepare;
Biber::Config->setoption('sortdebug', 1) if grep { $_ eq '--sortdebug' } @flags;
my $out = $biber->get_output_obj;
my $enc = Biber::Config->getoption('output_encoding') || 'UTF-8';
open(my $fh, ">:encoding($enc)", $outfile) or die "$outfile: $!";
$out->set_output_target($fh);
$out->output;
