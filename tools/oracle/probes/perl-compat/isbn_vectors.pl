#!/usr/bin/env perl
# Differential vectors for perl_compat::isbn (Business::ISBN 3.012 with
# Business::ISBN::Data 20260218.001 = data/isbn/RangeMessage.xml,
# Business::ISSN 1.008, Business::ISMN 1.205). Deterministic.
#
#   vendor/perl/bin/perl tools/oracle/probes/perl-compat/isbn_vectors.pl \
#       > crates/perl-compat/tests/data/isbn_vectors.json
use v5.24;
use strict;
use warnings;
no warnings;
use JSON::PP;
use Business::ISBN;
use Business::ISSN;
use Business::ISMN;

srand(4242);
sub pick { $_[int rand @_] }
sub digits { join '', map { int rand 10 } 1 .. $_[0] }

my %data = %Business::ISBN::country_data;
my @inputs = ('', 'abc', '0-306-40615-2', '0306406153', '978-0-306-40615-7', '9780306406158',
  '979-10-90636-07-1', '9791234567896', '0-8044-2957-X', '0-8044-2957-x', 'isbn 0 8044 2957 X',
  '978030640615X', 'X306406152', '12345678901', '123456789012345', '99999-99-9-9', '979-0-1234-5678-5',
  '9786111234567', '6111234567', "\x{FF10}306406152", 'ISBN-13: 978-1-4028-9462-6', '0-9752298-0-X');
for my $ean ('978', '979') {
  for my $g (sort keys %{ $data{$ean} }) {
    next if $g =~ /^_/;
    my $ranges = $data{$ean}{$g}[1];
    for (1 .. 3) {
      my @r = @$ranges;
      my $pub;
      if (@r) {
        my $i = 2 * int(rand(@r / 2));
        my ($lo, $hi) = @r[$i, $i + 1];
        $pub = $lo;
        $pub = $hi if rand() < 0.3;
      }
      else { $pub = digits(2) }
      my $len = 9 - length($g) - length($pub);
      next if $len < 1;
      my $body = $ean . $g . $pub . digits($len);
      my $isbn = Business::ISBN->new($body . '0');
      next unless $isbn;
      $isbn->fix_checksum;
      my $s = $isbn->isbn;
      push @inputs, $s;
      push @inputs, substr($s, 3) if $ean eq '978';       # its ISBN-10 shape
      push @inputs, $isbn->as_string // $s;
      substr(my $bad = $s, -1, 1) = (substr($s, -1) + 1) % 10;
      push @inputs, $bad;
    }
  }
}
for (1 .. 800) { push @inputs, pick('978', '979', '', '97') . digits(pick(9, 10, 12, 13)) . pick('', 'X', '-') }

my @isbnv;
for my $in (@inputs) {
  my $i = Business::ISBN->new($in);
  my %r = (in => "$in", defined => ($i ? JSON::PP::true : JSON::PP::false));
  if ($i) {
    $r{valid} = 0 + $i->{valid};
    $r{isbn} = $i->isbn;
    $r{as_string} = $i->as_string;
    $r{group_code} = $i->group_code;
    $r{publisher_code} = $i->publisher_code;
    if ($i->is_valid) {
      my $t = $i->as_isbn13;
      $r{isbn13} = $t ? $t->isbn : undef;
      $r{isbn13_str} = $t ? $t->as_string : undef;
      my $u = $i->as_isbn10;
      $r{isbn10} = $u ? $u->isbn : undef;
      $r{isbn10_str} = $u ? $u->as_string : undef;
    }
  }
  push @isbnv, \%r;
}

my @issn_in = ('', '0317-8471', '0317-8472', '0378-5955', '2049-3630', '1050-124X', '1050-124x',
  'issn 0317 8471', '03178471X', '123', '12345678', "\x{FF10}3178471");
for (1 .. 600) { push @issn_in, digits(7) . pick(0 .. 9, 'X', 'x') . pick('', '', ' ') }
my @issnv;
for my $in (@issn_in) {
  my $i = Business::ISSN->new($in);
  push @issnv, { in => "$in", defined => ($i ? JSON::PP::true : JSON::PP::false),
    ($i ? (valid => ($i->is_valid ? JSON::PP::true : JSON::PP::false), as_string => scalar($i->as_string)) : ()) };
}

my @ismn_in = ('', 'M-2306-7118-7', 'M-2306-7118-8', 'M230671187', 'm 2306 7118 7', '9790230671187',
  'M0750000000', 'M3000000000', 'M0420000000', 'M0000000000');
for (1 .. 1500) { push @ismn_in, pick('M', 'm', 'M-', '') . digits(9) }
my @ismnv;
for my $in (@ismn_in) {
  my $i = Business::ISMN->new($in);
  push @ismnv, { in => "$in", defined => ($i ? JSON::PP::true : JSON::PP::false),
    ($i ? (valid => 0 + $i->is_valid, as_string => scalar($i->as_string), country => scalar($i->country)) : ()) };
}

print JSON::PP->new->utf8->canonical->encode({ isbn => \@isbnv, issn => \@issnv, ismn => \@ismnv,
  max_group_len => $Business::ISBN::MAX_COUNTRY_CODE_LENGTH });
