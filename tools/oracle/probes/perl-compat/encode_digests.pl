#!/usr/bin/env perl
# Exhaustive Encode behaviour per encoding, as SHA-256 digests, for
# crates/perl-compat/tests/encode_exhaustive.rs (tests/data/encode_digests.json).
#
# Per encoding:
#   cb     encode(ENC, ALL, sub { sprintf "<%X>", $_[0] })   ALL = every code
#          point (no surrogates) joined with "\n"
#   layer  ALL printed through a PerlIO :encoding(ENC) layer (1024-byte
#          chunks; BB-152)
#   dec    for every 1- and 2-byte sequence, and 3-byte sequences starting
#          with 0x8E/0x8F: the File::Slurper-style strict read result
#          ("hex code points" or "ERR <message>")
# Plus `aliases`: Encode::find_encoding over many candidate names (with
# biber's define_alias calls).
#
# Usage: vendor/perl/bin/perl tools/oracle/probes/perl-compat/encode_digests.pl \
#          > crates/perl-compat/tests/data/encode_digests.json
use strict;
use warnings;
no warnings qw(utf8 surrogate nonchar non_unicode once);
use Encode qw(find_encoding encode decode);
use Encode::Alias;
use Digest::SHA qw(sha256_hex);
use JSON::PP;

define_alias('ansinew'        => 'cp1252');
define_alias('applemac'       => 'MacRoman');
define_alias('applemacce'     => 'MacCentralEurRoman');
define_alias('next'           => 'MacRoman');
define_alias('x-mac-roman'    => 'MacRoman');
define_alias('x-mac-centeuro' => 'MacCentralEurRoman');
define_alias('x-mac-cyrillic' => 'MacCyrillic');
define_alias('x-nextstep'     => 'MacRoman');
define_alias('x-ascii'        => 'ascii');
define_alias('lutf8'          => 'UTF-8');
define_alias('utf8x'          => 'UTF-8');

my @skip = qw(UTF-7 gsm0338 MIME-B MIME-Header MIME-Header-ISO_2022_JP MIME-Q);
my %skip = map { $_ => 1 } @skip;
my @encs = grep { !$skip{$_} } Encode->encodings(':all');
@encs = grep { my $x = $_; grep { $_ eq $x } @ARGV } @encs if @ARGV;

my $all = join("\n", map { chr } grep { $_ < 0xD800 || $_ > 0xDFFF } 0 .. 0x10FFFF);

sub read_layer {
  my ($enc, $bytes) = @_;
  local $PerlIO::encoding::fallback = Encode::STOP_AT_PARTIAL() | Encode::FB_CROAK();
  my $r = eval {
    open(my $fh, "<:raw:encoding($enc)", \$bytes) or die "open: $!";
    local $/;
    my $s = <$fh>;
    $s // '';
  };
  if (!defined $r) {
    my $e = $@;
    $e =~ s/ at \S+ line \d+.*//s;
    return "ERR $e";
  }
  return join(' ', map { sprintf '%X', ord } split //, $r);
}

my %out;
for my $enc (@encs) {
  my $o = find_encoding($enc) or die $enc;
  my %d;
  $d{cb} = sha256_hex(encode($enc, $all, sub { sprintf '<%X>', $_[0] }));
  my $buf = '';
  {
    local $SIG{__WARN__} = sub { };
    open(my $fh, ">:encoding($enc)", \$buf) or die "$enc: $!";
    print $fh $all;
    close $fh;
  }
  $d{layer} = sha256_hex($buf);
  my $h = Digest::SHA->new(256);
  my @seqs = (map { chr } 0 .. 255);
  for my $a (0 .. 255) { push @seqs, map { chr($a) . chr($_) } 0 .. 255 }
  for my $a (0x8E, 0x8F) {
    for my $b (0xA1 .. 0xFE) { push @seqs, map { chr($a) . chr($b) . chr($_) } 0xA1 .. 0xFE }
  }
  local $SIG{__WARN__} = sub { };
  for my $s (@seqs) {
    $h->add(unpack('H*', $s) . ':' . read_layer($enc, $s) . "\n");
  }
  $d{dec} = $h->hexdigest;
  $out{encodings}{$o->name} = \%d;
  print STDERR "$enc\n";
}

my @cands;
for my $n (Encode->encodings(':all')) {
  push @cands, $n, lc $n, uc $n, "x-$n", "x_$n", ($n =~ s/-/_/gr), ($n =~ s/-//gr),
    ($n =~ s/-/ /gr), "$n\@euro", "${n}-raw";
}
for my $n (1 .. 20) {
  push @cands, "latin$n", "latin-$n", "l$n", "iso-8859-$n", "iso8859-$n", "iso_8859_$n",
    "iso8859$n", "8859-$n", "iso-latin-$n", "ISO-8859-$n:1987";
}
for my $n (37, 424, 437, 500, 737, 775, 850 .. 875, 932, 936, 949, 950, 1006, 1026, 1047,
  1250 .. 1258, 65000, 65001, 20932, 1361) {
  push @cands, "cp$n", "CP-$n", "cp_$n", "windows-$n", "windows$n", "win$n", "ibm-$n",
    "IBM$n", "ms$n", "ms-$n", "cp $n";
}
push @cands, qw(
  sjis x-sjis shift_jis Shift-JIS SHIFT_JIS shiftjis MS_Kanji windows-31j csShiftJIS
  euc-jp EUC-JP eucjp ujis x-euc-jp jp-euc eucJP-ms euc-jp-ms iso-2022-jp ISO-2022-JP
  iso-2022-jp-1 iso-2022-jp-2 iso-2022-jp-3 jis JIS 7bit-jis csISO2022JP
  gb2312 GB2312 GB_2312-80 gb2312-raw gb2312raw GB2312-RAW euc-cn EUC-CN cn-euc
  x-euc-cn gbk GBK cp936 gb18030 GB18030 hz HZ HZ-GB-2312 iso-ir-165 gb12345
  euc-kr EUC-KR kr-euc x-euc-kr uhc x-uhc windows-949 ks_c_5601-1987 ksc5601 johab
  iso-2022-kr ISO-2022-KR big5 Big5 BIG-5 big5-eten big5et tca-big5 big5-hkscs
  big5hk hkscs-big5 big5-2003 euc-tw cp950
  UTF-8 utf-8 utf8 UTF8 utf-8-strict UTF-8-STRICT unicode-1-1-utf-7 UTF-7 utf7
  UTF-16 utf16 UTF16 utf-16be UTF16BE utf-16le UTF-16-LE UTF-32 utf32 UTF-32BE
  utf32le UCS-2 ucs2 UCS-2BE ucs-2le UCS2LE UCS-4 ucs4 UCS-4BE UCS-4LE iso-10646-1
  ascii ASCII us-ascii US-ASCII usascii x-ascii C c ISO646-US iso-646-us 646 ansi_x3.4-1968
  ANSI_X3.4-1968 ansinew applemac applemacce next x-mac-roman x-mac-centeuro
  x-mac-cyrillic x-nextstep lutf8 utf8x macintosh mac macce MacRoman macroman
  mac-roman x-mac-greek mac_cyrillic koi8-r KOI8R koi8 r koi8-u koi8u koi8_u koi8-f
  viscii hp-roman8 roman8 hp-greek8 kana8 thai8 tis-620 TIS620 tis620-2533
  winlatin1 winlatin2 wincyrillic winbaltic WinGreek winturkish winhebrew winarabic
  winvietnamese cyrillic arabic greek hebrew thai iso-8859-8-i iso_8859-8-I
  iso-ir-100 nextstep dingbats symbol null ascii-ctrl posix-bc AdobeStandardEncoding
  MIME-B mime-q MIME-Header gsm0338 Internal Unicode Guess bogus x xyz-euc-jp
  zh_CN.GB2312 ja_JP.eucJP ja_JP.SJIS en_US.UTF-8 de_DE@euro latin1@euro
  utf-8@euro sjis@euro Latin_1 latin 1 iso 8859 1 iso-8859 1 a_b_c ISO_8859_1
);
my %al;
for my $c (@cands) {
  my $o = find_encoding($c);
  $al{$c} = $o ? $o->name : undef;
}
$out{aliases} = \%al;
print JSON::PP->new->canonical->pretty->encode(\%out);
