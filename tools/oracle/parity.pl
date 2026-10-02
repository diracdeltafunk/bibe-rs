#!/usr/bin/perl
# Parity check: PAR biber binary (`biber` on PATH) vs oracle O3
# (tools/oracle/biber-src) on the same inputs.  Compares .bbl byte-for-byte
# after normalising only the hash-random per-name option order
# (docs/research/05 §5.4), compares exit codes and WARN/ERROR lines of the
# .blg, and records wall times.
#
# Usage: /usr/bin/perl tools/oracle/parity.pl [case-name-regex]
# Work dir: vendor/perl-build/parity/{par,src}/<case>/
use strict;
use warnings;
use File::Basename qw(basename dirname);
use File::Copy qw(copy);
use File::Path qw(make_path remove_tree);
use Cwd qw(abs_path);
use Time::HiRes qw(time);

my $root = abs_path(dirname(abs_path($0)) . '/../..');
my $work = "$root/vendor/perl-build/parity";
my $tdata = "$root/vendor/biber/t/tdata";
my $perf = "$root/scratch/perf";
my $filter = shift // '.';
my %runner = (par => ['biber'], src => ["$root/tools/oracle/biber-src"]);
my @common = ('--noconf');

# cases: name => [srcdir, bcf, [extra files...]]
my @cases;
push @cases, ['perf-small', $perf, 'small.bcf', ['ex.bib']];
for my $bcf (sort glob("$tdata/*.bcf")) {
  my $b = basename($bcf, '.bcf');
  push @cases, ["tdata-$b", $tdata, "$b.bcf", 'ALL'];
}
push @cases, ['perf-n10', $perf, 'n10.bcf', ['big10.bib']];
# Bonus: \nocite{*} over biblatex-examples.bib (found via kpsewhich) for 5 real styles.
for my $bcf (sort glob("$root/scratch/output/corpus/ex-*.bcf")) {
  my $b = basename($bcf, ".bcf");
  push @cases, ["corpus-$b", dirname($bcf), "$b.bcf", []];
}
@cases = grep { $_->[0] =~ /$filter/ } @cases;

sub slurp { my $f = shift; open my $h, '<:raw', $f or return undef; local $/; <$h> }

sub norm_bbl {
  my $s = shift // return undef;
  # \name{author}{2}{<opts>}{%   and   {{<opts>}{%  : sort comma-separated opts
  $s =~ s/(\\name\{[^}]*\}\{\d+\}\{)([^}]*)(\}\{%)/$1 . join(',', sort split m{,}, $2) . $3/ge;
  $s =~ s/^(\s*\{\{)([^}]*)(\}\{%)$/$1 . join(',', sort split m{,}, $2) . $3/gem;
  return $s;
}

sub blg_msgs {
  my $s = slurp(shift) // return '';
  my @m;
  for (split /\n/, $s) {
    next unless /^\[\d+\]\s+\S+>\s+(WARN|ERROR)\s+-\s+(.*)$/;
    my ($lvl, $msg) = ($1, $2);
    $msg =~ s{\Q$work\E/(par|src)/}{<W>/}g;
    push @m, "$lvl - $msg";
  }
  return join("\n", @m);
}

my @rows;
for my $c (@cases) {
  my ($name, $dir, $bcf, $extra) = @$c;
  my %r;
  for my $who (qw(par src)) {
    my $d = "$work/$who/$name";
    remove_tree($d); make_path($d);
    my @files = $extra eq 'ALL' ? (grep { -f } glob("$dir/*")) : (map { "$dir/$_" } ($bcf, @$extra));
    copy($_, "$d/" . basename($_)) or die "copy $_: $!" for @files;
    my $t0 = time;
    my $pid = fork // die;
    if (!$pid) {
      chdir $d or die;
      open STDOUT, '>', "$d/stdout.txt"; open STDERR, '>&', \*STDOUT;
      exec @{$runner{$who}}, @common, $bcf; exit 127;
    }
    waitpid($pid, 0);
    my $rc = $? >> 8;
    my $dt = time - $t0;
    (my $stem = $bcf) =~ s/\.bcf$//;
    $r{$who} = { rc => $rc, t => $dt, bbl => scalar(slurp("$d/$stem.bbl")), blg => blg_msgs("$d/$stem.blg") };
  }
  my ($p, $s) = @r{qw(par src)};
  my $raw_eq = defined $p->{bbl} && defined $s->{bbl} && $p->{bbl} eq $s->{bbl};
  my $norm_eq = (norm_bbl($p->{bbl}) // "\0none") eq (norm_bbl($s->{bbl}) // "\0none");
  my $status = $p->{rc} != $s->{rc} ? 'RC-DIFF'
             : !defined $p->{bbl} && !defined $s->{bbl} ? 'NO-BBL'
             : $raw_eq ? 'IDENTICAL'
             : $norm_eq ? 'SAME-NORM'
             : 'DIFF';
  my $blgeq = $p->{blg} eq $s->{blg} ? "same"
    : join("\n", sort split /\n/, $p->{blg}) eq join("\n", sort split /\n/, $s->{blg}) ? "reord"
    : "differ";
  if ($status eq 'DIFF') {
    open my $a, '>:raw', "$work/$name.par.bbl"; print $a norm_bbl($p->{bbl}) // '';
    open my $b, '>:raw', "$work/$name.src.bbl"; print $b norm_bbl($s->{bbl}) // '';
  }
  if ($blgeq ne "same") {
    open my $a, '>', "$work/$name.par.msgs"; print $a $p->{blg};
    open my $b, '>', "$work/$name.src.msgs"; print $b $s->{blg};
  }
  push @rows, [$name, $status, $p->{rc}, $s->{rc}, $blgeq, $p->{t}, $s->{t}];
  printf "%-32s %-10s rc=%d/%d warn/err=%-6s par=%7.2fs src=%7.2fs\n", @{$rows[-1]};
}
my %n; $n{$_->[1]}++ for @rows;
my ($tp, $ts) = (0, 0); $tp += $_->[5], $ts += $_->[6] for @rows;
printf "\nTOTAL %d cases: %s; wall par=%.1fs src=%.1fs\n", scalar(@rows),
  join(', ', map { "$_=$n{$_}" } sort keys %n), $tp, $ts;
