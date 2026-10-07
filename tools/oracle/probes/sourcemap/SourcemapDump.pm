package SourcemapDump;
# Record biber's source map engine at work, for differential tests of
# bibers' pipeline/sourcemap.rs (crates/bibers/tests/sourcemap.rs).
#
# Usage (see run.sh): perl -I<this dir> -MSourcemapDump biber ... with
# BIBERS_SMAP_OUT=<file>. One JSON line per call of
# Biber::Input::file::bibtex::create_entry:
#   { section, key, datasource,
#     before: { entry, allkeys, allkeys_nocite, cite[], nocite[], citekeys[],
#               rkeys[], uniq_counter, mapuniqval },
#     after:  { skip, entry, new: [{citekey, entry}], citekeys[], nocite[],
#               rkeys[], warnings[], debug[] } }
# where an entry is { type, key, fields: [[name, value], ...] } as
# Text::BibTeX holds it (names raw, values NFD as `get` returns them).
# `new` is sorted by citekey (biber creates them in hash order).
#
# The map engine runs with debug logging enabled (only inside the engine, so
# biber otherwise runs normally) to capture its debug messages, and
# suniqid as used by maploopreplace is replaced by bibers' deterministic
# $MAPUNIQ generator (DC-018): base62(10^16 + n), n = 1, 2, ... per run.

use strict;
use warnings;
use JSON::PP;
use Encode qw(decode);
use Unicode::Normalize qw(NFD);

my $out = $ENV{BIBERS_SMAP_OUT};
my $json = JSON::PP->new->canonical(1)->utf8(1);
our $IN_MAP = 0;
my $COUNTER = 0;
my @debug;
my $cur;       # record of the current create_entry call
my $created;   # _create_entry calls seen in the current create_entry
my $warn_start;

sub mapuniq_id {
  use integer;
  my $n = shift;
  my @c = ('0'..'9', 'a'..'z', 'A'..'Z');
  my $v = 10000000000000000 + $n;
  my $s = '';
  while ($v > 0) { $s = $c[$v % 62] . $s; $v = $v / 62; }
  return $s;
}

sub str {
  my $s = shift;
  return undef unless defined $s;
  return utf8::is_utf8($s) ? $s : decode('UTF-8', $s);
}

sub dump_entry {
  my $e = shift;
  return undef unless $e;
  return {
    type   => scalar($e->type),
    key    => scalar($e->key),
    fields => [ map { [ str($_), (defined $e->{values}{$_} ? NFD(str($e->{values}{$_})) : undef) ] }
                @{$e->{fields}} ],
  };
}

sub keys_of { my $h = shift // {}; return [ sort keys %$h ] }

sub finish_mapping {
  my $w = $Biber::MASTER->{warnings} // [];
  return { warnings => [ @$w[$warn_start .. $#$w] ], debug => [ @debug ] };
}

INIT {
  return unless $out;
  require Biber;
  no strict 'refs';
  no warnings 'redefine';
  # bibtex.pm reads the data model when loaded: install the wrappers just
  # before the first fetch_data.
  my $fd = \&Biber::fetch_data;
  my $installed;
  *Biber::fetch_data = sub {
    install() unless $installed++;
    goto &$fd;
  };
}

sub install {
  require Biber::Input::file::bibtex;
  require Log::Log4perl::Logger;
  no strict 'refs';
  no warnings 'redefine';

  *{"Biber::Utils::suniqid"} = sub { return mapuniq_id(++$COUNTER) };

  my $isdebug = \&Log::Log4perl::Logger::is_debug;
  *Log::Log4perl::Logger::is_debug = sub { return 1 if $IN_MAP; goto &$isdebug };
  my $dbg = \&Log::Log4perl::Logger::debug;
  *Log::Log4perl::Logger::debug = sub {
    if ($IN_MAP) {
      shift;
      push @debug, join('', map { ref $_ eq 'CODE' ? $_->() : $_ } @_);
      return;
    }
    goto &$dbg;
  };

  my $ce = \&Biber::Input::file::bibtex::create_entry;
  *Biber::Input::file::bibtex::create_entry = sub {
    my ($key, $entry, $datasource, $smaps, $rkeys) = @_;
    my $secnum = $Biber::MASTER->get_current_section;
    my $section = $Biber::MASTER->sections->get_section($secnum);
    @debug = ();
    $created = [];
    $warn_start = scalar @{$Biber::MASTER->{warnings} // []};
    my $rec = {
      section => "$secnum", key => $key, datasource => $datasource,
      before => {
        entry => dump_entry($entry),
        allkeys => $section->{allkeys} ? 1 : 0,
        allkeys_nocite => $section->{allkeysnocite} ? 1 : 0,
        cite => keys_of($section->{cite_citekeys}),
        nocite => keys_of($section->{nocite_citekeys}),
        citekeys => [ @{$section->{citekeys} // []} ],
        rkeys => [ @$rkeys ],
        uniq_counter => $COUNTER,
        mapuniqval => $Biber::Utils::MAPUNIQVAL,
      },
    };
    $cur = $rec;
    my $r;
    {
      local $IN_MAP = 1;
      # biber redirects STDERR to a temp file here: record dies in the output.
      local $SIG{__DIE__} = sub {
        return if $^S;
        open my $fh, '>>:raw', $out or return;
        print $fh $json->encode({die => "$_[0]", key => $key}), "\n";
        close $fh;
      };
      $r = $ce->(@_);
    }
    my $after = $rec->{after} //= finish_mapping();
    $after->{citekeys} = [ @{$section->{citekeys} // []} ];
    $after->{nocite} = keys_of($section->{nocite_citekeys});
    $after->{rkeys} = [ @$rkeys ];
    $after->{skip} = @$created ? 0 : 1;
    $after->{entry} = @$created ? $created->[0]{entry} : undef;
    $after->{new} = [ sort { $a->{citekey} cmp $b->{citekey} } @$created[1 .. $#$created] ];
    $cur = undef;
    open my $fh, '>>:raw', $out or die "SourcemapDump: $out: $!";
    print $fh $json->encode($rec), "\n";
    close $fh;
    return $r;
  };

  my $cre = \&Biber::Input::file::bibtex::_create_entry;
  *Biber::Input::file::bibtex::_create_entry = sub {
    my ($k, $e) = @_;
    if ($cur and $IN_MAP) {
      $cur->{after} //= finish_mapping();
      push @$created, { citekey => $k, entry => dump_entry($e) } if $e;
    }
    local $IN_MAP = 0;
    return $cre->(@_);
  };
}

1;
