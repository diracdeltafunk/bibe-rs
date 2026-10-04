package BiberPhaseDump;
# Dump biber's internal state after each processing phase, as canonical JSON,
# so bibers can be tested phase by phase (docs/PLAN.md §6, M2).
#
# Usage (see run.sh): load into the source-built biber (oracle O3) with
#   perl -I<this dir> -MBiberPhaseDump <biber-src>/blib/script/biber ...
# and set BIBERS_DUMP_DIR=<dir>. After every wrapped phase method returns, a
# file <dir>/<NNN>-s<section>-<phase>[-<list>].json is written (plus an `init`
# dump of the section as set up from the .bcf, just before fetch_data) containing
#   { phase, section, list, state: {section, datalists, config_state, entry_options, preamble} }
#
# Biber itself runs completely normally; the dumper only reads.
#
# Canonicalisation:
# * hash keys are sorted; all defined scalars are dumped as JSON strings
#   (Perl doesn't distinguish reliably), undef as null;
# * blessed Biber::* objects become {"__class": "...", ...fields}; other
#   blessed objects (collators, XML nodes, ...) are opaque {"__class": X};
# * code refs -> "__CODE__", qr// -> "__REGEX__:<stringified>";
# * the random suniqid ids of Biber::Entry::Names / Biber::Entry::Name
#   objects are renamed to "__nl<N>__" / "__n<N>__" in order of first appearance in a
#   deterministic traversal of the dumped state (numbered per dump), wherever they appear (values and hash keys), so
#   dumps are reproducible and object sharing is visible as equal ids.

use strict;
use warnings;
use Scalar::Util qw(blessed reftype refaddr);
use JSON::PP;

my $dir = $ENV{BIBERS_DUMP_DIR};
my $seq = 0;
my %idmap;          # random id -> canonical id
my %count = (nl => 0, n => 0);

sub canon_id {
  my ($id, $prefix) = @_;
  return $idmap{$id} //= '__' . $prefix . (++$count{$prefix}) . '__';
}

# First pass: assign canonical ids in deterministic traversal order.
sub collect {
  my ($x, $seen) = @_;
  my $rt = reftype($x) // return;
  return if $seen->{refaddr($x)}++;
  my $cls = blessed($x) // '';
  if ($rt eq 'HASH') {
    if ($cls eq 'Biber::Entry::Names' and defined $x->{id}) { canon_id($x->{id}, 'nl') }
    if ($cls eq 'Biber::Entry::Name'  and defined $x->{id}) { canon_id($x->{id}, 'n') }
    return if $cls and $cls !~ /^Biber::/;
    collect($x->{$_}, $seen) for sort keys %$x;
  }
  elsif ($rt eq 'ARRAY') {
    return if $cls and $cls !~ /^Biber::/;
    collect($_, $seen) for @$x;
  }
  elsif ($rt eq 'SCALAR' or $rt eq 'REF') {
    collect($$x, $seen);
  }
}

sub str {
  my $s = shift;
  return exists $idmap{$s} ? $idmap{$s} : $s;
}

sub conv {
  my ($x, $depth) = @_;
  return undef unless defined $x;
  my $rt = reftype($x);
  return str("$x") unless $rt;
  die "BiberPhaseDump: structure too deep" if $depth > 200;
  my $cls = blessed($x);
  if ($cls and $cls eq 'Regexp') { return "__REGEX__:$x" }
  if ($rt eq 'CODE') { return '__CODE__' }
  if ($cls and $cls !~ /^Biber::/) { return { __class => $cls } }
  if ($rt eq 'HASH') {
    my %h = map { (str($_) => conv($x->{$_}, $depth + 1)) } keys %$x;
    $h{__class} = $cls if $cls;
    return \%h;
  }
  if ($rt eq 'ARRAY') {
    my @a = map { conv($_, $depth + 1) } @$x;
    return $cls ? { __class => $cls, __array => \@a } : \@a;
  }
  if ($rt eq 'SCALAR' or $rt eq 'REF') { return { __scalarref => conv($$x, $depth + 1) } }
  return "__$rt\__";
}

my $json = JSON::PP->new->canonical(1)->utf8(1)->pretty(1);

sub entry_options {
  my $secnum = shift;
  my $all = $Biber::Config::CONFIG->{options}{biblatex}{ENTRY} // {};
  my %out;
  for my $key (keys %$all) {
    next unless exists $all->{$key}{$secnum};
    $out{$key} = $all->{$key}{$secnum};
  }
  return \%out;
}

sub dump_state {
  my ($biber, $phase, $list) = @_;
  return unless $dir;
  my $secnum = $biber->get_current_section;
  my $section = $biber->sections->get_section($secnum);
  my $lists = [ grep { defined } @{ $biber->datalists->get_lists_for_section($secnum) // [] } ];
  my $state = {
    section      => $section,
    datalists    => $lists,
    config_state => $Biber::Config::CONFIG->{state},
    # Per-entry biblatex options (set e.g. from the `options` field during
    # fetch_data), restricted to this section: {citekey => {opt => value}}.
    entry_options => entry_options($secnum),
    # $biber->{preamble}: all @preamble strings so far (global, cumulative
    # across sections).
    preamble     => $biber->{preamble},
  };
  # Number ids per dump (not per run), so a dump's ids depend only on the
  # state it contains and can be reproduced from that state alone.
  %idmap = ();
  %count = (nl => 0, n => 0);
  collect($state, {});
  my $out = {
    phase   => $phase,
    section => "$secnum",
    list    => defined $list ? $list->get_name : undef,
    state   => conv($state, 0),
  };
  my $name = sprintf('%03d-s%s-%s', ++$seq, $secnum, $phase);
  if (defined $list) {
    (my $ln = $list->get_name) =~ s/[^A-Za-z0-9_.-]+/_/g;
    $name .= "-$ln";
  }
  open my $fh, '>:raw', "$dir/$name.json" or die "BiberPhaseDump: $dir/$name.json: $!";
  print $fh $json->encode($out);
  close $fh;
}

# Section-level phases (Biber::prepare) and per-list phases (process_lists).
my @SECTION_PHASES = qw(
  fetch_data process_citekey_aliases instantiate_dynamic resolve_alias_refs
  resolve_xdata cite_setmembers preprocess_sets calculate_interentry
  process_interentry validate_datamodel postprocess_sets process_entries_static
  process_lists
);
my @LIST_PHASES = qw(
  process_entries_pre uniqueness process_visible_names process_entries_post
  process_entries_final generate_sortdataschema generate_sortinfo sort_list
  generate_contextdata
);

INIT {
  return unless $dir;
  require Biber;
  no strict 'refs';
  no warnings 'redefine';
  for my $p (@SECTION_PHASES) {
    my $orig = \&{"Biber::$p"};
    *{"Biber::$p"} = sub {
      # The section as set up from the .bcf, before any processing.
      dump_state($_[0], 'init', undef) if $p eq 'fetch_data';
      my @r = wantarray ? $orig->(@_) : scalar $orig->(@_);
      dump_state($_[0], $p, undef);
      return wantarray ? @r : $r[0];
    };
  }
  for my $p (@LIST_PHASES) {
    my $orig = \&{"Biber::$p"};
    *{"Biber::$p"} = sub {
      my @r = wantarray ? $orig->(@_) : scalar $orig->(@_);
      dump_state($_[0], $p, $_[1]);
      return wantarray ? @r : $r[0];
    };
  }
}

1;
