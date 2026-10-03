#!/usr/bin/env perl
# Differential vectors for perl_compat::uri (URI 5.34 `URI->new($s)->as_string`
# on character strings, as biber calls it). Deterministic.
#
#   vendor/perl/bin/perl tools/oracle/probes/perl-compat/uri_vectors.pl \
#       > crates/perl-compat/tests/data/uri_vectors.json
use v5.24;
use strict;
use warnings;
no warnings;
use utf8;
use JSON::PP;
use URI;

srand(777);
sub pick { $_[int rand @_] }

my @fixed = ('', ' ', 'http://example.com/a b', '<URL:http://x.org/é>', ' <URL:http://x.org/é> ',
  '<http://x.org/>', '"http://x.org/"', "<http://x.org/>\n", "\"<http://x.org/>\"", "<\"http://x.org/\">",
  'http://bücher.de/x', 'http://BÜCHER.de/x', 'http://user:pw@bücher.de:8080/p?q=1#f',
  'http://xn--bcher-kva.de/', 'http://xn--bü.de/', 'http://.bücher.de/', 'http://bücher..de/',
  'http://bücher.de./', 'http://a b.de/', 'http://ü' . ('a' x 70) . '.de/', 'http://x.org/100%',
  'http://x.org/%5B%5d', 'http://[::1]:80/', 'http://%5B::1%5D/', 'mailto:a%5Bb%5D@x.org',
  'urn:isbn:978 0', 'urn:x-y:é', 'data:text/plain,a b%5B', 'irc://irc.example.org/#chan',
  'ircs://x/#a b', 'irc:/#x', 'irc://x/%23a', 'irc://x/%23', 'ircs:/%23b', 'file:///tmp/a b', 'ftp://ü@x/', 'HTTP://Ü.DE/', 'Http://x/ü',
  'foo bar', 'x:y z', '1http://x', 'http:', 'http://', 'http:///x', 'http://ü:١٢/', 'http://x:12ü/',
  "http://a\nb\@ü/", "http://ü\n/", 'geo:foo', 'geo:90,10', 'geo:-90.0,10.5;crs=wgs84;u=12.5;x=y',
  'geo:52.1,4.3', 'geo:52,4,10', 'geo:90,4;u=1.', 'geo:1,2;crs=;x', 'news:comp.lang.perl',
  'ssh://ü/', 'telnet://日本.jp/', 'http://日本。jp/', 'ws://ß.de/', 'sip:ü@x', 'x-y.z+w:é',
  "\x{A0}http://x/\x{A0}", "\x{2003}http://x/", 'http://x/{a}|^`\\', "http://x/\x{1F600}",
  'https://dx.doi.org/10.1000/xyz<123>', 'doi:10.1000/a b', 'http://x/~a?b=c&d=e;f', "http://x/\t",
  'ldapi://ü/', 'sqlite:a%5B', 'mailto:%5bx', 'otpauth://totp/ü', 'smb://ü/', 'pop://ü/');
my @atoms = ('http://', 'https://', 'ftp://', 'irc://', 'mailto:', 'urn:', 'geo:', 'Foo:', '//',
  'ü', 'é', 'ß', '日本', 'a', 'Z', '0', '9', '.', '-', '_', '~', ' ', '%', '%20', '%5B', '%5d', '[',
  ']', '@', ':', '/', '?', '#', '&', '=', '+', '$', ',', ';', '!', '*', "'", '(', ')', '<', '>', '"',
  '{', '}', '|', '\\', '^', '`', "\x{1F600}", "\x{301}", "\x{A0}", "\x{663}", 'xn--', '80', 'URL:');
my @in = @fixed;
for (1 .. 1500) { push @in, join('', map { pick(@atoms) } 1 .. 1 + int rand 9) }

my @v;
for my $s (@in) {
  my $x = $s;
  utf8::upgrade($x);
  my $r = eval { URI->new($x)->as_string };
  push @v, { in => $s, out => (defined $r ? $r : undef), err => ($@ ? ($@ =~ /^(.*?) at /s)[0] : undef) };
}
print JSON::PP->new->utf8->canonical->encode(\@v);
