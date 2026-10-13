#!/usr/bin/env perl
# Validity verdicts of libxml2 (XML::LibXML::RelaxNG, as biber's
# Biber::Utils::validate_biber_xml uses it) for differential testing of
# crates/relaxng.
#
#   vendor/perl/bin/perl verdicts.pl SCHEMA.rng FILE...      # or FILE list on stdin with '-'
#
# Prints "FILE<TAB>valid|invalid|error" per file ("error": not well-formed
# or the parser died). With VERBOSE=1, libxml2's messages go to stderr.
use strict;
use warnings;
use XML::LibXML;

my $schema_file = shift @ARGV or die "usage: verdicts.pl SCHEMA.rng FILE...\n";
my $schema = XML::LibXML::RelaxNG->new(location => $schema_file);
my @files = @ARGV;
if (@files == 1 and $files[0] eq '-') {
  @files = map { chomp; $_ } <STDIN>;
}
$| = 1;
foreach my $f (@files) {
  my $parser = XML::LibXML->new();
  $parser->line_numbers(1);
  my $doc = eval { $parser->load_xml(location => $f) };
  unless ($doc) {
    print "$f\terror\n";
    next;
  }
  eval { $schema->validate($doc) };
  if ($@) {
    print "$f\tinvalid\n";
    print STDERR (ref($@) ? $@->as_string : $@) if $ENV{VERBOSE};
  }
  else {
    print "$f\tvalid\n";
  }
}
