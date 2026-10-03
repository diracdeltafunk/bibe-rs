#!/usr/bin/env perl
# Differential vectors for perl_compat::regex: biber's own Biber::Utils::ireplace
# and imatch, `split(/$S/, ...)` (xsvsep) and `s/$re//gxms` (nosort & co.),
# run in oracle O3. Deterministic.
#
#   vendor/perl/bin/perl -Ivendor/perl-build/biber-src/blib/lib \
#       tools/oracle/probes/perl-compat/regex_vectors.pl \
#       > crates/perl-compat/tests/data/regex_vectors.json
use v5.24;
use strict;
use warnings;
no warnings;
use utf8;
use JSON::PP;
use Biber::Utils qw(ireplace imatch);

srand(31337);
sub pick { $_[int rand @_] }
local $SIG{__WARN__} = sub { };

my @patterns = ('b', '(b)', '(b)(z)?', 'B', '(\w+)', '(\W)', '\s+', '(\S+)', '\d', '(\d+)', 'x*', 'a*',
  '^', '$', '\b', '\B', '(?<=a)b', '(?<!a)b', 'b(?=c)', 'b(?!c)', '(\w)\1', '(?<n>b)', '(?i)B',
  '\p{Lu}', '(\p{Ll}+)', '\p{L}', '\P{L}', '\p{Dash}', '\p{Pd}', '\p{Dia}', '\pL', '\pM', '\X',
  '\x{263A}', '\N{U+263B}', '\x41', '[[:alpha:]]+', '[[:punct:]]', '[^a-z]', '[\w-]+', '[^\W\d]',
  '[\s\S]', '.', '.+', '(.)(.)', 'a|b', '(a)|(b)', '(?:a)(b)?', '(a+)+b', '^\s+|\s+$', '\.\s*',
  '\Q.\E', '\Uc', 'é', '(?i)É', 'ß', '(?i)ss', '\h', '\v', '\R', '\K', 'a\Kb', '(a)(?|(b)|(c))',
  '[[:upper:]]', '\z', '\Z', '\A', '(?x) a b ', '(?s).', '(?m)^.', '\\\\', '\$1', '{', 'x{2}', 'a{,2}',
  '(?^i:B)', '(?a)\w', '(?u)\w', '\bvon\b', 'v(on|an) ', '\p{Latin}', '\p{Greek}', '\p{IsAlpha}',
  '\p{L&}', '\p{XPosixPunct}', '\p{Han}', '\p{InBasicLatin}', '[α-ω]', '(?i)[α-ω]', '\N{LATIN SMALL LETTER B}',
  '\b{wb}', '(?{ 1 })', '(', '[', 'a**', '\1', '(?<a>.)(?<a>.)', '\g{-1}', '(.)\g1', '\o{142}', '\142',
  '(?#c)b', '\cA', '[\Q]', '\p{XPosixAlpha}', '\p{PosixPunct}', '\p{^L}', '\P{IsLu}', '[\p{XPosixPunct}x]',
  '\p{Space}+', '\p{Word}+', '\p{Alnum}', '\p{XDigit}', '\p{Blank}', '\p{Cntrl}', '\p{IsL}', '\p{Is_Lu}',
  '\p{posix_digit}', '(?a)\d', '(?a)\s', '(?a)\bx', '\d+', '[^\d\s]', '0', '');
my @subjects = ('abc', 'aBc', 'a1b2c3', '  a , b ,c ', 'Hello World', 'éàü ÉÀÜ', 'straße STRASSE',
  "a\nb\nc\n", 'ab', 'aab', '', 'x', 'Ludwig van Beethoven', 'Jean-Paul Sartre', "a\x{263A}b\x{263B}c",
  "e\x{301}a\x{308}", 'αβγ ΑΒΓ', '中文 测试', 'a.b.c', 'ı İ i I', "\x{1F1E9}\x{1F1EA}x", '0', 'a²b',
  "x\x{180E}y", 'a_b-c', "a\tb\x{A0}c\x{2003}d");
my @replaces = ('X', '', '[$1]', '$&', '<$2>', '$1$1', '\u$1', '\U$&\E!', '\L$&', '-$`-$\'-', '$+',
  '$^N', '$-[0]:$+[0]', '@-/@+', '${1}x', '$x', 'a@b.c', '"', 'a\\', '@{[ uc $& ]}', '\Q$&', '\t',
  '\x{2014}', '\N{U+41}', '$0', '$#+', '$10', '\$1', '$1[0]', '\u\L$&', '\lABC', '$1 $2');

my (@ir, @im, @sp, @rm);
for my $p (@patterns) {
  for my $s (@subjects) {
    for my $ci (0, 1) {
      next if $ci && rand() < 0.6;
      my $r = pick(@replaces);
      my $out = eval { ireplace($s, $p, $r, $ci) };
      push @ir, { v => $s, p => $p, r => $r, ci => $ci ? JSON::PP::true : JSON::PP::false,
        out => (defined $out ? "$out" : undef), err => ($@ ? JSON::PP::true : JSON::PP::false) };
      for my $neg (0, 1) {
        next if $neg && rand() < 0.7;
        my @m = eval { imatch($s, $p, $neg, $ci) };
        my $err = $@ ? 1 : 0;
        push @im, { v => $s, p => $p, neg => $neg ? JSON::PP::true : JSON::PP::false,
          ci => $ci ? JSON::PP::true : JSON::PP::false,
          out => [map { defined $_ ? "$_" : undef } @m], err => $err ? JSON::PP::true : JSON::PP::false };
      }
    }
    my @f = eval { my $S = qr/$p/; split(/$S/, $s) };
    push @sp, { v => $s, p => $p, out => [map { defined $_ ? "$_" : undef } @f], err => ($@ ? JSON::PP::true : JSON::PP::false) };
    my $x = $s;
    eval { my $re = qr/$p/; $x =~ s/$re//gxms };
    push @rm, { v => $s, p => $p, out => "$x", err => ($@ ? JSON::PP::true : JSON::PP::false) };
  }
}
# all replacement strings against a fixed match
my @t6 = ('@-', '@+', '$-[1]', '$+[0]', '${\ "x"}', '$x{a}', '$#-', '$#+', '$#x', '$1->[0]', '$&[0]',
  '$1{x}', '${1}[0]', '$1:x', '$1::x', '$::x', '$x::y', '${x}', '${ 1 }', '$ 1', '$', 'a$', '$1$2',
  '\\$1', '\\@x', '@', 'a@', '@1', '@x[0]', '@{x}', '$x[1+1]', '$x[$1]', '$x[', '${', '$1{', '\\', 'a\\',
  '\\u\\LABC', '\\L\\uabc', '\\Uab\\Lcd', '\\Qa.b\\E.', '\\Fß', '\\x{', '\\x4g', '\\x{263a}', '\\0', '\\012', '\\c@',
  '\\e', '\\a', '\\z', '\\$&', '$&$&', '$`$\'', '${^MATCH}', '${^PREMATCH}', '$^W', '$;', '$,', '$"', '$@',
  '$1 $2', '#{x}', '$#{x}', '$x->{a}', '$x->[0]', '$x->foo', '$1->', '$+{n}', '$-{n}', '%+', '$:', '$^', '$~',
  '$11', '$01', '$١', '${١}', '$ab_c1', '$ü', '$x::', '$a::b::c', "\$1\n", '$1 ', "a\x{263a}b", '\\N{U+41}\\N{U+0042}',
  '\\N{ U+41 }', '\\o{101}', '\\101', '\\1', '\\8', '\\x', '\\xg', '\\cA', '\\c', 'a"', '\\"', '"', '$1\\', '@{[1+1]}',
  '${\\(1)}', '\\l\\U$1', '\\U\\l$1', '\\L\\U$1', 'x\\Ey', '\\E', '\\Q$&\\E', '\\Q\\U$1\\E\\E.', '$1\\U$1',
  '$=', '$-', '$%', '$|', '$?', '$.', '$/', '$\\', '$_', '$0', '$[', '$]', '$*', '$<', '$(', '$^A', '${^N}',
  '\\u', '\\u$2x', '\\U', 'a\\Q', '\\Q\\Q$&', '\\L\\L$&', '\\F\\U$&', '\\Q\\L$&', '\\N{LATIN SMALL LETTER A}',
  '@$x', '@::x', '$$x', '${$x}', '@{ x }', '$x [0]', '$1 [0]', "\\\n", '\\U$1\\Lx\\Uy', '$+[1]', '$-[2]', '$+[-1]',
  '\\x{1F600}', '\\o{}', '\\x{}', '$#', '$#$x', '$+', '$$1', '\\uab\\Ecd', '\\u$2\\E$1', '\\ua\\Ub',
  '\\Ua\\ub', '\\u$2$1x', '\\lAB\\QC.D', 'x\\u', '\\u\\Q.ab', '\\Q\\u.ab');
for my $r (@t6) {
  for my $p ('(B)', '(B)(z)?', '((a)B)') {
    my $out = eval { ireplace('aBc', $p, $r, 0) };
    $out =~ s/\b\d{3,}\b/PID/g if $r =~ /\$\$|\$[<>(]/;   # process ids
    push @ir, { v => 'aBc', p => $p, r => $r, ci => JSON::PP::false, out => (defined $out ? "$out" : undef),
      err => ($@ ? JSON::PP::true : JSON::PP::false) };
  }
}
binmode(STDOUT, ":raw");
print JSON::PP->new->utf8->canonical->encode({ ireplace => \@ir, imatch => \@im, split => \@sp, remove => \@rm });
