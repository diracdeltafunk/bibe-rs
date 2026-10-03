#!/usr/bin/env perl
# Turn the hand-written experiment files in cases/ into oracle requests
# (JSON lines for vectors.pl). Every request carries an "id" that
# docs/spec/btparse.md cites.
#   cases/<x>.txt with "### ID ..." headers  -> parse requests (id = ID)
#   cases/names*.txt (one name per line)     -> name requests  (id = <file>:<line>)
#   cases/split_list*.txt (one per line)     -> list requests with "and"
use strict; use warnings;
use FindBin;
use JSON::PP;
my $J = JSON::PP->new->utf8->canonical;
my $dir = "$FindBin::Bin/cases";
for my $f (sort glob("$dir/*.txt")) {
  my ($base) = $f =~ m{([^/]+)\.txt$};
  open my $fh, '<:encoding(UTF-8)', $f or die;
  local $/;
  my $in = <$fh>;
  if ($base =~ /^names/ or $base =~ /^split_list/) {
    my $n = 0;
    for my $l (split /\n/, $in) {
      $n++;
      my %r = (id => "$base:$n");
      if ($base =~ /^names/) { $r{op} = 'name'; $r{s} = $l; $r{sanitise} = JSON::PP::true }
      else { $r{op} = 'list'; $r{s} = $l; $r{d} = 'and' }
      print $J->encode(\%r), "\n";
    }
    next;
  }
  my @cases = split /^### /m, $in;
  shift @cases;
  for my $c (@cases) {
    my ($hdr, $body) = split /\n/, $c, 2;
    $body //= '';
    $body =~ s/\n\z// unless $body =~ /\n\n\z/;
    my ($id) = $hdr =~ /^(\S+)/;
    print $J->encode({ id => $id, op => 'parse', text => $body }), "\n";
  }
}
