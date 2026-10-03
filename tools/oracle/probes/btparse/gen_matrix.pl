#!/usr/bin/env perl
# Generate cases/matrix.txt: every special character in every lexer/parser position.
use strict; use warnings;
my %pos = (
  P1 => ['@', 'misc{k, title = {x}}'],          # right after @
  P2 => ['@misc', '{k, title = {x}}'],           # after type, before open
  P2b => ['@misc', 'k, title = {x}}'],           # after type, instead of open
  P3 => ['@misc{', 'k, title = {x}}'],          # key position
  P4 => ['@misc{k', ', title = {x}}'],          # after key
  P5 => ['@misc{k, ', 'title = {x}}'],          # field-name position
  P6 => ['@misc{k, title', ' = {x}}'],          # after field name
  P7 => ['@misc{k, title = ', '{x}}'],          # value position
  P8 => ['@misc{k, title = {x}', '}'],          # after value
  P9 => ['@misc{k, title = {x} # ', '{y}}'],    # after hash
  P10 => ['@string{', 'm = {x}}'],              # macro-name position
  P11 => ['@preamble{', '{x}}'],                # preamble value position
  P12 => ['@comment', '{x}'],                   # after comment type
);
my %ch = (LB => '{', RB => '}', LP => '(', RP => ')', DQ => '"', HS => '#', CM => ',', EQ => '=',
          PC => "%\n", AT => '@', SQ => "'", BS => '\\', DG => '7', NM => 'zz', NL => "\n");
for my $p (sort keys %pos) {
  for my $c (sort keys %ch) {
    my ($a, $b) = $pos{$p}->@*;
    print "### M-$p-$c\n$a$ch{$c}$b\n\@misc{after, title = {ok}}\n";
  }
}
