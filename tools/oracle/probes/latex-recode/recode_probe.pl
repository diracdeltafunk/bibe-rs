#!/usr/bin/env perl
# Differential probe for crates/latex-recode (oracle O3).
#
# Run with the O3 perl:
#   vendor/perl/bin/perl -Ivendor/perl-build/biber-src/blib/lib recode_probe.pl
#
# Protocol (line-oriented; payloads are hex-encoded UTF-8 so any code point is safe):
#   "init <decodeset> <encodeset> [<recodedata path>]"  -> "ok"
#   "d <hex>"    latex_decode($s)                  (default: NFD at the end)
#   "dn <hex>"   latex_decode($s, normalize => 0)
#   "e <hex>"    latex_encode($s)
#   "x <hex>"    \X clusters joined with U+0001
#   "nfd <hex>" / "nfc <hex>"
# Results are hex of Perl's internal (lax) UTF-8, so non-Unicode code points
# produced by \char survive. A Perl exception yields "!<hex of message>".
use v5.24;
use strict;
no warnings;
use Encode;
use Log::Log4perl qw(:easy);
Log::Log4perl->easy_init($FATAL);
use Biber::Config;
use Biber::LaTeX::Recode;
use Unicode::Normalize;

$| = 1;
binmode STDIN, ':raw';
binmode STDOUT, ':raw';

sub unhex { my $b = pack('H*', $_[0]); utf8::decode($b); $b }
sub tohex { my $s = $_[0]; utf8::upgrade($s); utf8::encode($s); unpack('H*', $s) }

# Slurp first: init_sets runs kpsewhich through IPC::Run3 for --recodedata,
# which re-opens STDIN and would discard PerlIO's read-ahead buffer.
my @lines = <STDIN>;
for my $line (@lines) {
  chomp $line;
  my ($op, @a) = split / /, $line;
  if ($op eq 'init') {
    my ($d, $e, $rd) = @a;
    Biber::Config->setoption('recodedata', $rd) if defined $rd;
    Biber::LaTeX::Recode->init_sets($d, $e);
    print "ok\n";
    next;
  }
  my $s = unhex($a[0] // '');
  my $r = eval {
    if    ($op eq 'd')   { tohex(latex_decode($s)) }
    elsif ($op eq 'dn')  { tohex(latex_decode($s, normalize => 0)) }
    elsif ($op eq 'e')   { tohex(latex_encode($s)) }
    elsif ($op eq 'x')   { tohex(join("\x{1}", $s =~ /(\X)/g)) }
    elsif ($op eq 'nfd') { tohex(NFD($s)) }
    elsif ($op eq 'nfc') { tohex(NFC($s)) }
    else { die "bad op $op" }
  };
  if ($@) { print '!', tohex("$@"), "\n" } else { print "$r\n" }
}
