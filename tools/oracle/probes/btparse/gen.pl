#!/usr/bin/env perl
# Grammar-based random generator of btparse oracle requests (JSON lines for
# vectors.pl).
#   gen.pl SEED COUNT [parse|name|list|all]  > req.jsonl
# Mostly well-formed BibTeX with frequent character-level mutations (so that
# error recovery is exercised), names with prefixes/braces/special characters/
# Unicode case/commas, and name lists with delimiter variants.
use strict; use warnings;
use utf8;
use JSON::PP;

my ($seed, $count, $what) = @ARGV;
$seed //= 1; $count //= 1000; $what //= 'all';
srand($seed);
my $J = JSON::PP->new->utf8->canonical;
binmode STDOUT;

sub pick { $_[int rand @_] }
sub chance { rand() < $_[0] }

# ---------------------------------------------------------------- bib text
my @types = qw(article book misc inproceedings ARTICLE Book online thesis x-type);
my @fnames = qw(title author editor year month note pages journal Title AUTHOR url doi keywords
                publisher date ids crossref xdata options);
my @macros = qw(jan feb dec acm ieee undefd foo Foo FOO bar);
my @ws = (' ', ' ', ' ', '', "\n", "\n  ", "\t", '  ');

sub ws { pick(@ws) }

sub word {
  my @w = qw(The Mating Habits of Adult Foobars a b x y Smith Doe von der la {\'e}t{\'e}
             {ABC} $x^2$ 50% @ \& ~ -- éa Ł ß 李 и \\textit{x} {} {{x}} A.  and AND);
  return pick(@w);
}

sub braced_text {
  my $d = shift // 0;
  my $n = int rand 6;
  my @p;
  for (1 .. $n) {
    if ($d < 3 && chance(0.15)) { push @p, '{' . braced_text($d + 1) . '}' }
    else { push @p, word() }
  }
  my $s = '';
  for my $p (@p) { $s .= pick(' ', ' ', ' ', '  ', "\n", "\n\t", "\t", '') . $p }
  $s .= pick('', ' ', "\n") if chance(0.2);
  return $s;
}

sub simple_value {
  my $r = rand;
  if ($r < 0.45) { return '{' . braced_text() . '}' }
  if ($r < 0.65) { my $t = braced_text(); $t =~ s/"/''/g; return '"' . $t . '"' }
  if ($r < 0.8) { return pick(1999, 2020, 0, 12, 7, '0012') }
  return pick(@macros);
}

sub value {
  my $v = simple_value();
  while (chance(0.2)) { $v .= ws() . '#' . ws() . simple_value() }
  return $v;
}

sub key {
  return pick('k1', 'Key:2020', 'doe-2001a', 'a.b', 'X_Y', '123', 'é1', 'k@x', 'a/b+c');
}

sub field { return pick(@fnames) . ws() . '=' . ws() . value() }

sub entry {
  my $r = rand;
  my $open = chance(0.85) ? ['{', '}'] : ['(', ')'];
  if ($r < 0.1) {
    my @defs = map { pick(@macros, 'm1', 'm2') . ws() . '=' . ws() . value() } 1 .. 1 + int rand 3;
    return '@' . pick('string', 'STRING', 'String') . ws() . $open->[0] . join(',' . ws(), @defs)
      . (chance(0.2) ? ',' : '') . $open->[1];
  }
  if ($r < 0.15) { return '@' . pick('preamble', 'PREAMBLE') . $open->[0] . value() . $open->[1] }
  if ($r < 0.2) {
    my $t = braced_text();
    return '@' . pick('comment', 'Comment') . ws() . $open->[0] . $t . $open->[1];
  }
  my @f = map { field() } 0 .. int rand 5;
  my $s = '@' . pick(@types) . ws() . $open->[0] . ws() . key() . ws() . ',';
  $s .= ws() . join(',' . ws(), @f) if @f;
  $s .= ',' if chance(0.3);
  $s .= ws() . $open->[1];
  return $s;
}

sub bibfile {
  my $s = '';
  $s .= pick('junk ', "% comment\n", '') if chance(0.3);
  for (1 .. 1 + int rand 4) {
    $s .= entry();
    $s .= pick("\n", "\n\n", ' ', '', " % c\n", "\njunk\n", "\n% x \@misc{no}\n");
  }
  # mutations
  if (chance(0.45)) {
    for (1 .. 1 + int rand 3) {
      my $pos = int rand(length($s) + 1);
      my $m = rand;
      if ($m < 0.55) {
        substr($s, $pos, 0) = pick('{', '}', '(', ')', '"', '#', ',', '=', '%', '@', "'", '\\', "\n", 'zz', '7', "\x01");
      }
      elsif ($m < 0.85) { substr($s, $pos, 1) = '' if $pos < length $s }
      else { substr($s, $pos, 0) = substr($s, int rand(length $s), 1 + int rand 4) }
    }
  }
  return $s;
}

# ---------------------------------------------------------------- names
my @first = ('John', 'J.', 'Jean-Pierre', 'A.', 'Mary~Ann', 'Ab', '{\\\'E}mile', 'Émile', 'Łukasz',
             'X', 'Al', 'J.-P.', '{Jr}', 'Ūx', '李李', 'ﬁx');
my @von = ('von', 'van', 'de', 'la', 'der', 'du', 'ébauche', 'ąb', 'łx', 'αβ', 'ии', 'ꬰx', 'ǆx', 'de~la',
           'd\'', 'van~der');
my @last = ('Smith', 'Doe', 'Beethoven', '{Barnes and Noble}', 'Vall{\\\'e}e', '{\\relax D}oe',
            'Jones-Smith', 'O\'Neil', '{van}', 'Poussin', 'Ålund', 'Øre', 'ǅx', '{\\ss}x', 'b', 'Ab');
my @jr = ('Jr.', 'III', 'Jr', 'fils', 'jr');

sub nametok {
  my $r = rand;
  return pick(@first) if $r < 0.35;
  return pick(@von) if $r < 0.55;
  return pick(@last) if $r < 0.9;
  return pick('{x}', '{}', '\\o', '{\\o}x', '7', '-x', 'others', '{A B}', 'a}b', '{a');
}

sub name {
  my $r = rand;
  my @t = map { nametok() } 0 .. int rand 4;
  my $s;
  if ($r < 0.5) { $s = join(' ', @t) }
  elsif ($r < 0.8) { $s = pick(@von, '') . ' ' . pick(@last) . ', ' . join(' ', @t) }
  elsif ($r < 0.92) { $s = pick(@last) . ', ' . pick(@jr) . ', ' . join(' ', @t) }
  else {
    # arbitrary comma soup (irregular cases)
    $s = join(pick(',', ' ', ', ', ' , ', ',,'), @t) . pick('', ',', ', ,');
    $s = ',' . $s if chance(0.2);
  }
  $s =~ s/^ +//;
  $s = pick('others', 'Others', '{others}') if chance(0.03);
  $s =~ s/ / pick(' ', ' ', '  ', "\t") /ge if chance(0.2);
  return $s;
}

sub namelist {
  my $d = pick('and', 'and', 'and', 'und', ';', 'AND');
  my @n = map { name() } 0 .. int rand 4;
  my $s = '';
  for my $i (0 .. $#n) {
    $s .= ($i ? pick(" $d ", " \U$d\E ", " $d ", "  $d ", " $d  ", "\t$d ", "~$d~", " $d", " {$d} ") : '') . $n[$i];
  }
  $s = "$d $s" if chance(0.05);
  $s .= " $d" if chance(0.05);
  $s = "{$s}" if chance(0.05);
  return ($s, $d);
}

for my $i (1 .. $count) {
  my $kind = $what eq 'all' ? pick('parse', 'parse', 'name', 'list') : $what;
  my %r = (id => "g$seed-$i", op => $kind);
  if ($kind eq 'parse') { $r{text} = bibfile() }
  elsif ($kind eq 'name') { $r{s} = name(); $r{sanitise} = JSON::PP::true }
  else { ($r{s}, $r{d}) = namelist() }
  print $J->encode(\%r), "\n";
}
