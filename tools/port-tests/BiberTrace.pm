# BiberTrace: instrumentation for running biber's own t/*.t under oracle O3.
#
# Loaded with `perl -MBiberTrace t/foo.t` (see tools/port-tests/trace.sh). It
# records, as JSON lines in $BIBERTRACE_OUT/events.jsonl:
#
#   * every call the test script makes into a Biber::* sub (setoption,
#     setblxoption, parse_ctrlfile, prepare, set_sortingtemplatename, accessors,
#     utility functions, ...), with serialised arguments and return value;
#   * every Test::More / Test::Differences assertion, with got/expected/name,
#     the test number and the .t line;
#   * after every prepare() whose output object is a Biber::Output::bbl, the
#     complete in-process .bbl (with sortdebug lines) in $BIBERTRACE_OUT/bbl/N.bbl.
#
# Calls made by biber internally are not recorded (the wrapper `goto`s the
# original sub unless the immediate caller is the test script), so the
# instrumentation is transparent to biber itself.
package BiberTrace;
use strict;
use warnings;
use B ();
use Scalar::Util qw(blessed reftype refaddr);
use JSON::PP ();
use Cwd ();
use File::Path ();

our $OUT;
our $FH;
our $SEQ = 0;
our $PREP = 0;
our $IN_ASSERT = 0;
our %WRAPPED;
our $TESTFILE = $0;

my $json = JSON::PP->new->utf8->canonical->allow_nonref;

sub emit {
  my $ev = shift;
  $ev->{seq} = ++$SEQ;
  print $FH $json->encode($ev), "\n";
}

my %BIG = map {$_ => 1} qw(Biber Biber::Section Biber::Sections Biber::DataList
  Biber::DataLists Biber::Entries Biber::Output::bbl Biber::Output::bibtex
  Biber::Output::biblatexml Biber::Output::bblxml Biber::Output::dot
  Biber::Output::base Biber::DataModel Biber::LangTags Biber::Config
  Biber::Input::file::bibtex XML::LibXML::Document Log::Log4perl::Logger);

sub ser {
  my ($x, $depth, $seen) = @_;
  $depth //= 0;
  $seen //= {};
  return undef unless defined $x;
  my $r = ref $x;
  return "$x" unless $r;
  return {__regex => "$x"} if $r eq 'Regexp';
  return {__code => 1} if $r eq 'CODE';
  return {__ref => 'GLOB'} if $r eq 'GLOB';
  return {__scalarref => ser($$x, $depth + 1, $seen)} if $r eq 'SCALAR' or $r eq 'REF';
  if (my $cls = blessed($x)) {
    if ($cls eq 'Biber::Entry') {
      my $k = eval { $x->get_field('citekey') };
      return {__obj => $cls, key => $k};
    }
    if ($cls eq 'Biber::Entry::Names') {
      my $id = eval { $x->get_id };
      my @names = eval { map { ser($_, $depth + 1, $seen) } $x->names->@* };
      return {__obj => $cls, id => $id, names => \@names, (eval { $x->get_morenames } ? (morenames => 1) : ())};
    }
    if ($BIG{$cls} or $cls =~ /^(XML|Log|IO|File|Unicode)::/) {
      my %o = (__obj => $cls);
      if ($cls eq 'Biber::DataList') { $o{name} = eval { $x->get_name }; $o{section} = eval { $x->get_section }; $o{sortingtemplatename} = eval { $x->get_sortingtemplatename }; }
      $o{number} = eval { $x->number } if $cls eq 'Biber::Section';
      return \%o;
    }
  }
  return {__cycle => 1} if $seen->{refaddr($x)};
  return {__deep => 1} if $depth > 12;
  local $seen->{refaddr($x)} = 1;
  my $rt = reftype($x);
  my $v;
  if ($rt eq 'ARRAY') {
    $v = [ map { ser($_, $depth + 1, $seen) } @$x ];
  }
  elsif ($rt eq 'HASH') {
    $v = { map { $_ => ser($x->{$_}, $depth + 1, $seen) } keys %$x };
  }
  else {
    $v = "$x";
  }
  if (my $cls = blessed($x)) {
    return {__obj => $cls, value => $v};
  }
  return $v;
}

sub test_caller {
  # Is the sub (whose wrapper is running) being called directly from the test file?
  my @c = caller(1);
  return unless @c;
  return $c[1] eq $TESTFILE ? $c[2] : undef;
}

sub wrap_sub {
  my ($pkg, $name, $orig) = @_;
  my $full = "${pkg}::$name";
  my $special = ($full eq 'Biber::prepare' or $full eq 'Biber::prepare_tool');
  my $w = sub {
    my @c = caller(0);
    unless (defined $c[1] and $c[1] eq $TESTFILE) {
      goto &$orig;
    }
    my $line = $c[2];
    my $args = ser([@_]);
    my $want = wantarray;
    my (@r, $r);
    if ($want) { @r = $orig->(@_) }
    elsif (defined $want) { $r = $orig->(@_) }
    else { $orig->(@_) }
    my $ev = {ev => 'call', sub => $full, line => $line, args => $args,
              ctx => ($want ? 'list' : defined($want) ? 'scalar' : 'void')};
    $ev->{ret} = $want ? ser(\@r) : ser($r);
    if ($special) {
      $PREP++;
      $ev->{prep} = $PREP;
      $ev->{cwd} = Cwd::getcwd();
      dump_output($_[0], $PREP) if $full eq 'Biber::prepare';
    }
    emit($ev);
    return $want ? @r : $r;
  };
  my $proto = prototype($orig);
  Scalar::Util::set_prototype(\&$w, $proto) if defined $proto;
  no strict 'refs';
  no warnings 'redefine';
  *{$full} = $w;
}

sub dump_output {
  my ($biber, $n) = @_;
  my $out = eval { $biber->get_output_obj } or return;
  return unless $out->isa('Biber::Output::bbl') and ref($out) eq 'Biber::Output::bbl';
  my $file = "$OUT/bbl/$n.bbl";
  my $enc = Biber::Config->getoption('output_encoding') || 'UTF-8';
  my $sd = Biber::Config->getoption('sortdebug');
  local $out->{output_target};
  local $out->{output_target_file} = $file;
  open(my $fh, ">:encoding($enc)", $file) or die "$file: $!";
  $out->{output_target} = $fh;
  Biber::Config->setoption('sortdebug', 1);
  my $ok = eval { $out->output; 1 };
  Biber::Config->setoption('sortdebug', $sd);
  emit({ev => 'dump_error', prep => $n, error => "$@"}) unless $ok;
}

sub wrap_package {
  my $pkg = shift;
  no strict 'refs';
  my $stash = \%{"${pkg}::"};
  foreach my $name (sort keys %$stash) {
    next if $name =~ /^(BEGIN|END|INIT|CHECK|UNITCHECK|DESTROY|AUTOLOAD|import|unimport|can|isa|VERSION|__ANON__)$/;
    next if $name =~ /::$/;
    my $glob = $stash->{$name};
    next unless ref(\$glob) eq 'GLOB';
    my $code = *{$glob}{CODE} or next;
    my $p = prototype($code);
    next if defined($p) and $p eq '';   # constants
    my $gv = B::svref_2object($code)->GV;
    my $origin = eval { $gv->STASH->NAME } // '';
    next unless $origin =~ /^(Biber|Class::Accessor|main)\b/ or $origin eq '';
    next if $origin eq 'main';
    next if $WRAPPED{"${pkg}::$name"}++;
    wrap_sub($pkg, $name, $code);
  }
}

sub wrap_assert {
  my ($pkg, $name) = @_;
  no strict 'refs';
  my $orig = \&{"${pkg}::$name"};
  return unless defined &$orig;
  my $w = sub {
    if ($IN_ASSERT) { goto &$orig }
    my @c = caller(0);
    local $IN_ASSERT = 1;
    my $tb = Test::More->builder;
    my $before = $tb->current_test;
    my $args = ser([@_]);
    my $res = $orig->(@_);
    my $after = $tb->current_test;
    emit({ev => 'assert', fn => $name, line => $c[2], file => $c[1],
          args => $args, ok => ($res ? 1 : 0), tests => [$before + 1 .. $after]});
    return $res;
  };
  my $proto = prototype($orig);
  Scalar::Util::set_prototype(\&$w, $proto) if defined $proto;
  no warnings 'redefine';
  *{"${pkg}::$name"} = $w;
}

{
  $OUT = $ENV{BIBERTRACE_OUT} or die "BIBERTRACE_OUT not set";
  $OUT = Cwd::abs_path($OUT);
  File::Path::make_path("$OUT/bbl");
  open($FH, '>', "$OUT/events.jsonl") or die "$OUT/events.jsonl: $!";
  $FH->autoflush(1);

  require Test::More;
  require Test::Differences;
  foreach my $a (qw(ok is isnt like unlike is_deeply cmp_ok)) {
    wrap_assert('Test::More', $a);
  }
  foreach my $a (qw(eq_or_diff eq_or_diff_text eq_or_diff_data)) {
    wrap_assert('Test::Differences', $a);
  }

  require Biber;
  require Biber::Utils;
  require Biber::Constants;
  require Biber::Output::bbl;
  require Biber::Output::bibtex;
  require Biber::Output::biblatexml;
  require Biber::LaTeX::Recode;
  require Biber::LangTag;
  require Biber::Entry::Name;
  require Biber::Entry::Names;
  wrap_all();
  *CORE::GLOBAL::require = sub {
    my $f = shift;
    return CORE::require($f) if $f =~ /^v?[\d._]+$/;
    my $already = exists $INC{$f};
    my $r = CORE::require($f);
    wrap_all() if !$already and $f =~ m{^Biber/};
    return $r;
  };
  emit({ev => 'start', file => $TESTFILE});
}

sub wrap_all {
  no strict 'refs';
  my @pkgs;
  my $walk;
  $walk = sub {
    my $p = shift;
    foreach my $k (keys %{"${p}::"}) {
      next unless $k =~ /^(\w+)::$/;
      my $sub = "${p}::$1";
      push @pkgs, $sub;
      $walk->($sub);
    }
  };
  push @pkgs, 'Biber';
  $walk->('Biber');
  my $skip = $ENV{BIBERTRACE_SKIP};
  @pkgs = grep { !defined($skip) or $_ !~ /$skip/ } @pkgs;
  wrap_package($_) foreach sort @pkgs;
}

END {
  emit({ev => 'end'}) if $FH;
}

1;
