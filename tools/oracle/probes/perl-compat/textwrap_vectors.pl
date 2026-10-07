#!/usr/bin/env perl
# Differential vectors for perl_compat::textwrap (Text::Wrap 2024.001 `wrap`
# at its defaults, as biber's --wraplines uses it). Deterministic.
#
#   vendor/perl/bin/perl tools/oracle/probes/perl-compat/textwrap_vectors.pl \
#       > crates/perl-compat/tests/data/textwrap_vectors.json
use v5.24;
use strict;
use warnings;
no warnings;
use utf8;
use JSON::PP;
use Text::Wrap;

srand(4242);
sub pick { $_[int rand @_] }

my @atoms = ('a', 'bb', 'word', 'longerword', 'x' x 25, ' ', ' ', ' ', '  ', "\t", "\n",
  "e\x{301}", "\x{301}", "\x{a0}", "\x{202f}", "\r\n", ',', '\\field{x}{', '}', "\x{3000}",
  "\x{1F600}", 'ü', ' ,}}');
my @out;
my @fixed = ('', ' ', "\n", "abc\n", "a b c\n\n", "\x{301}abc def", '      \\field{title}{A long title here}');
for my $i (0 .. 599) {
  my $text = $i < @fixed ? $fixed[$i] : join('', map { pick(@atoms) } 1 .. (1 + int rand 30));
  my $cols = pick(2 .. 12, 20, 30, 40, 80);
  my ($ip, $xp) = @{ pick(['      ', '      '], ['', ''], ['  ', ''], ["\t", ' ']) };
  local $Text::Wrap::columns = $cols;
  my $r = eval { wrap($ip, $xp, $text) };
  push @out, { cols => "$cols", ip => $ip, xp => $xp, in => $text,
               (defined $r ? (out => $r) : (err => "$@")) };
}
print JSON::PP->new->canonical(1)->utf8(1)->pretty(1)->encode(\@out);
