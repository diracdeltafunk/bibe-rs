#!/usr/bin/env perl
# Unicode::Collate oracle for crates/ucollate differential tests.
#
#   vendor/perl/bin/perl tools/oracle/probes/ucollate/oracle.pl < requests > responses
#
# One response line per request line:
#   C<TAB>JSON   create the current collator. JSON: {"locale": STR|null, "new": {...},
#                "change": {...}} -> Unicode::Collate::Locale->new(locale => …, %new)
#                ->change(%change). Option values: strings, null (undef) or arrays of
#                numbers. Reply "ok" or "err MESSAGE".
#   K<TAB>HEX…   sort key of the string made of these code points (hex, space separated).
#                Reply "HEXKEY<TAB>viewSortKey" or "err MESSAGE".
#   L<TAB>NAME   reply the getlocale() of a collator for locale NAME ("~" = undef).
#   V<TAB>KEY    reply the option value change() reports for KEY on the current collator
#                (without changing it), or "~" for undef.
use strict;
use warnings;
use Unicode::Collate 1.31;
use Unicode::Collate::Locale;
use JSON::PP;

binmode STDOUT, ':raw';
$| = 0;
my $json = JSON::PP->new;
my $coll = Unicode::Collate::Locale->new();
while (my $line = <STDIN>) {
    chomp $line;
    my ($cmd, $arg) = split /\t/, $line, 2;
    $arg //= '';
    if ($cmd eq 'C') {
        my $spec = $json->decode($arg);
        my $c = eval {
            my $x = Unicode::Collate::Locale->new(locale => $spec->{locale}, %{ $spec->{new} || {} });
            $x->change(%{ $spec->{change} || {} }) if $spec->{change};
            $x;
        };
        if ($c) { $coll = $c; print "ok\n" }
        else { (my $e = $@) =~ s/ at \S+ line \d+.*//s; $e =~ s/\s+/ /g; print "err $e\n" }
    } elsif ($cmd eq 'K') {
        my $s = join '', map { chr hex } split ' ', $arg;
        my ($k, $v) = eval { ($coll->getSortKey($s), $coll->viewSortKey($s)) };
        if (defined $k) { print unpack('H*', $k), "\t$v\n" }
        else { (my $e = $@) =~ s/\s+/ /g; print "err $e\n" }
    } elsif ($cmd eq 'L') {
        my $c = Unicode::Collate::Locale->new(locale => ($arg eq '~' ? undef : $arg));
        print $c->getlocale, "\n";
    } elsif ($cmd eq 'V') {
        my %old = $coll->change();
        my $v = $coll->{$arg};
        print defined $v ? (ref $v eq 'ARRAY' ? 'ARRAY' : ref $v ? 'CODE' : $v) : '~', "\n";
    } else {
        print "err unknown command\n";
    }
}
