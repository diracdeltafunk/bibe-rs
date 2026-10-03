#!/usr/bin/env perl
# Oracle probe for bibers-core's configuration front end (milestone M1).
#
# Runs biber 2.22's own code (oracle O3) and dumps its post-parse state as
# canonical JSON, for differential tests against bibers.
#
# Usage (with the O3 perl, see tools/oracle/README.md "Run"):
#   vendor/perl/bin/perl -Ivendor/perl-build/biber-src/blib/lib \
#     tools/oracle/probes/bibers-core/probe.pl MODE < jobs.jsonl > results.jsonl
#
# MODE getopt: each input line is a JSON array (an argv). Runs bin/biber's
#   GetOptions call and option post-processing (the code between
#   `use Getopt::Long` and `Biber->new`, extracted from bin/biber at run
#   time) and prints {argv, opts, rest, exit, stderr}.
# MODE config: each input line is {"id", "dir", "args"}. In a fork, chdirs
#   to dir, runs the same prelude on args (plus --nolog), then Biber->new,
#   parse_ctrlfile, postprocess_biber_opts and process_setup, and prints
#   {id, exit, log, state} where state mirrors bibers_core::bcf::dump_state.
#
# Every job runs in a forked child, so biber's process-global state never
# leaks between jobs.

BEGIN {
  *CORE::GLOBAL::exit = sub { die "EXIT:" . ($_[0] // 0) . "\n" };
}

use v5.24;
use strict;
use warnings;
use JSON::PP;
use File::Temp qw(tempfile);
use Scalar::Util qw(blessed reftype);
use Digest::MD5 ();
use Encode ();

my $BIBER_SRC = $ENV{BIBER_SRC} // '/Users/ben/bibe-rs/vendor/biber';
my $mode = shift @ARGV or die "usage: probe.pl getopt|config\n";

# Extract bin/biber's prelude.
my $prelude = do {
  open(my $fh, '<:encoding(UTF-8)', "$BIBER_SRC/bin/biber") or die "bin/biber: $!";
  local $/; my $s = <$fh>;
  my ($p) = $s =~ m/(use Getopt::Long qw\/:config no_ignore_case\/;.*?)# Create Biber object/s
    or die "cannot find prelude";
  $p;
};

require Biber;
require Biber::Utils;
require Biber::Output::bbl;
require Pod::Usage;

my $json = JSON::PP->new->canonical(1)->utf8(1);

# Stringify every scalar; keep structure.
sub canon {
  my ($v, $seen) = @_;
  return undef unless defined $v;
  if (my $r = ref $v) {
    my $t = reftype($v) // '';
    return '<CODE>' if $t eq 'CODE';
    return "$v" if $r eq 'Regexp';
    if ($t eq 'ARRAY') { return [ map { canon($_) } @$v ] }
    if ($t eq 'HASH') { return { map { $_ => canon($v->{$_}) } keys %$v } }
    if ($t eq 'SCALAR' || $t eq 'REF') { return canon($$v) }
    return "<$r>";
  }
  return "$v";
}

sub run_prelude {
  my @argv = @_;
  local @ARGV = @argv;
  local $0 = "$BIBER_SRC/bin/biber";  # pod2usage reads the POD from $0
  my $err = '';
  {
    local $SIG{__WARN__} = sub { $err .= $_[0] };
    local *STDERR;
    open(STDERR, '>', \my $buf) or die;
    local *STDOUT;
    open(STDOUT, '>', \$main::__stdout) or die;
    my $ok = eval "package main; no strict; use Pod::Usage; use List::AllUtils qw(first); use constant {EXIT_OK => 0, EXIT_ERROR => 2}; $prelude; \$main::__opts = \$opts; 1";
    $err .= $buf // '';
    if (!$ok) {
      my $e = $@;
      if ($e =~ /^EXIT:(\d+)/) { return (undef, [@ARGV], $1, $err) }
      return (undef, [@ARGV], "DIE", $err . $e);
    }
  }
  return ($main::__opts, [@ARGV], undef, $err);
}

sub job_getopt {
  my $argv = shift;
  $main::__stdout = '';
  my ($opts, $rest, $exit, $err) = run_prelude(@$argv);
  my $out = $main::__stdout // '';
  utf8::decode($out);
  return { argv => $argv, opts => canon($opts), rest => $rest, exit => $exit, stderr => $err,
           stdout => length($out) > 300 ? 'md5:' . Digest::MD5::md5_hex(Encode::encode_utf8($out)) : $out };
}

sub dm_dump {
  my $dm = shift;
  my %d;
  for my $k (qw(fieldsbyname fieldsbytype fieldsbyfieldtype fieldsbydatatype fieldsbyformat
                constants entrytypesbyname casemap helpers)) {
    $d{$k} = canon($dm->{$k} // {});
  }
  return \%d;
}

sub state_dump {
  my $biber = shift;
  no warnings 'once';
  my $C = $Biber::Config::CONFIG;
  my %s;
  my %bo = %{$C->{options}{biber}};
  delete $bo{bcf};
  $s{biber} = canon(\%bo);
  my $blx = $C->{options}{biblatex} // {};
  my %g = %{$blx->{GLOBAL} // {}};
  delete $g{datamodel};
  $s{blx} = { GLOBAL => canon(\%g), ENTRYTYPE => canon($blx->{ENTRYTYPE} // {}),
              ENTRY => canon($blx->{ENTRY} // {}) };
  my %bo2 = map { $_ => canon($Biber::Constants::CONFIG_BIBLATEX_OPTIONS{$_} // {}) }
    qw(GLOBAL ENTRYTYPE ENTRY NAMELIST NAME);
  $s{registry} = { optscope => canon(\%Biber::Constants::CONFIG_OPTSCOPE_BIBLATEX),
                   opttype => canon(\%Biber::Constants::CONFIG_OPTTYPE_BIBLATEX),
                   blxoptions => \%bo2 };
  $s{datafieldsets} = canon(\%Biber::Constants::DATAFIELD_SETS);
  $s{keyorder} = canon($C->{state}{keyorder} // {});
  $s{internalkeyorder} = canon($C->{state}{internalkeyorder} // {});
  if (my $secs = $biber->sections) {
    my @sections;
    foreach my $sec ($secs->get_sections->@*) {
      push @sections, {
        number => "" . $sec->number,
        datasources => canon($sec->{datasources}),
        citekeys => canon($sec->{citekeys}),
        orig_order_citekeys => canon($sec->{orig_order_citekeys}),
        allkeys => "" . ($sec->{allkeys} // 0),
        allkeys_nocite => "" . ($sec->{allkeysnocite} // 0),
        cite => [ sort keys %{$sec->{cite_citekeys}} ],
        nocite => [ sort keys %{$sec->{nocite_citekeys}} ],
        dkeys => canon($sec->{dkeys}),
        citecount => canon($sec->{citecount}),
        seenkeys => canon($sec->{seenkeys}),
      };
    }
    my @lists;
    my $sts = Biber::Config->getblxoption(undef, 'sortingtemplate');
    foreach my $l (($biber->datalists ? $biber->datalists->get_lists : [])->@*) {
      my $loc = '<none>';
      if ($l->{sortinitcollator}) {
        my $stn = $l->{sortingtemplatename};
        $loc = (ref $sts eq 'HASH' and defined $stn and $sts->{$stn}) ? $sts->{$stn}{locale} : undef;
      }
      push @lists, {
        section => canon($l->{section}),
        name => canon($l->{name}),
        type => canon($l->{type}),
        (map { $_ => canon($l->{$_}) } qw(sortingtemplatename sortingnamekeytemplatename
          uniquenametemplatename labelalphanametemplatename namehashtemplatename labelprefix)),
        filters => canon($l->{filters} // []),
        sortinit_locale => canon($loc),
      };
    }
    $s{doc} = { sections => \@sections, datalists => \@lists };
  }
  if (my $dm = $C->{dm}) { $s{dm} = dm_dump($dm) }
  $s{warnings} = canon($biber->{warnings} // []);
  return \%s;
}

sub job_config {
  my $job = shift;
  chdir $job->{dir} or return { id => $job->{id}, error => "chdir: $!" };
  my ($opts, $rest, $exit, $err) = run_prelude(@{$job->{args}}, '--nolog');
  return { id => $job->{id}, exit => $exit, stderr => $err } if defined $exit;
  local @ARGV = @$rest;
  my ($logfh, $logname) = tempfile();
  my $res = { id => $job->{id}, opts => canon($opts), rest => $rest };
  my $biber;
  open(my $saveout, '>&', \*STDOUT) or die;
  open(STDOUT, '>', $logname) or die;
  my $ok = eval {
    $biber = Biber->new(%$opts);
    my $out = Biber::Output::bbl->new();
    $biber->set_output_obj($out);
    my ($ofh, $oname) = tempfile();
    $out->set_output_target_file($oname);
    if (Biber::Config->getoption('tool')) { die "TOOLMODE\n" }
    $biber->parse_ctrlfile(Biber::Config->getoption('bcf'));
    Biber::Config->postprocess_biber_opts;
    $biber->process_setup;
    1;
  };
  my $e = $@;
  close(STDOUT); open(STDOUT, '>&', $saveout) or die;
  if (!$ok) {
    if ($e =~ /^EXIT:(\d+)/) { $res->{exit} = $1 }
    else { $res->{exit} = 'DIE'; $res->{die} = $e }
  }
  open(my $lfh, '<:encoding(UTF-8)', $logname); my @log = <$lfh>; chomp @log;
  $res->{log} = \@log;
  $res->{state} = state_dump($biber) if $biber;
  return $res;
}

binmode(STDIN, ':raw');
while (my $line = <STDIN>) {
  next unless $line =~ /\S/;
  my $job = $json->decode($line);
  pipe(my $r, my $w) or die;
  my $pid = fork() // die;
  if ($pid == 0) {
    close $r;
    my $res = $mode eq 'getopt' ? job_getopt($job) : job_config($job);
    print $w $json->encode($res);
    close $w;
    POSIX::_exit(0);
  }
  close $w;
  local $/; my $out = <$r>;
  waitpid($pid, 0);
  print $out, "\n";
}
use POSIX ();
