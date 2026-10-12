#!/usr/bin/env perl
# Differential vectors for bibers' XPath 1.0 evaluator (crates/bibers-core/
# src/xmldom/xpath.rs): evaluate each expression with XML::LibXML as biber
# does ($node->findnodes / findvalue / exists), on the entries of a fixed
# biblatexml document, and print JSON lines.
#
# Run with the O3 perl:
#   vendor/perl/bin/perl tools/oracle/probes/xpath/xpath_vectors.pl \
#     > crates/bibers-core/tests/data/xpath/vectors.jsonl
use strict;
use warnings;
use utf8;
use XML::LibXML;
use JSON::PP;

my $NS = 'http://biblatex-biber.sourceforge.net/biblatexml';
my $xml = <<"EOF";
<?xml version="1.0" encoding="UTF-8"?>
<bltx:entries xmlns:bltx="$NS" xmlns:o="http://example.org/o">
  <bltx:entry id="e1" entrytype="book">
    <bltx:names type="author" useprefix="true">
      <bltx:name gender="sm"><bltx:namepart type="family">Doe</bltx:namepart><bltx:namepart type="given"><bltx:namepart initial="J">John</bltx:namepart><bltx:namepart>Q</bltx:namepart></bltx:namepart></bltx:name>
      <bltx:name><bltx:namepart type="family">Roe</bltx:namepart></bltx:name>
    </bltx:names>
    <bltx:title>The <!-- c -->Title</bltx:title>
    <bltx:date>2001</bltx:date>
    <bltx:date type="orig"><bltx:start>1990</bltx:start><bltx:end>1992</bltx:end></bltx:date>
    <bltx:pages><bltx:list><bltx:item><bltx:start>1</bltx:start><bltx:end>5</bltx:end></bltx:item><bltx:item><bltx:start>10</bltx:start><bltx:end>12</bltx:end></bltx:item></bltx:list></bltx:pages>
    <o:title>other</o:title>
    <bltx:note>  spaced   note </bltx:note>
    <bltx:volume>3</bltx:volume>
  </bltx:entry>
  <bltx:entry id="e2" entrytype="misc"><bltx:title>Second</bltx:title><bltx:volume>12</bltx:volume></bltx:entry>
</bltx:entries>
EOF

my @exprs = (
  './bltx:title', './bltx:title/text()', '*', './*', 'node()', './@id', './@*', '@entrytype',
  "./bltx:names[\@type='author']", "./bltx:names[\@type='author']/bltx:name[2]",
  "./bltx:names[\@type='author']/bltx:name[1]/bltx:namepart[\@type='given']/bltx:namepart",
  "./bltx:names/bltx:name[last()]", "./bltx:names/bltx:name[position()=1]/\@gender",
  './bltx:date', './bltx:date[@type]', './bltx:date[not(@type)]', "./bltx:date[\@type='orig']/bltx:start",
  './/bltx:namepart', './/bltx:namepart[@initial]', './/bltx:item[2]/bltx:end',
  '(.//bltx:start)[2]', './bltx:pages//bltx:start | ./bltx:pages//bltx:end',
  './bltx:title/..', './bltx:title/../@id', './bltx:volume/following-sibling::*',
  './bltx:volume/preceding-sibling::*[1]', './bltx:title/following::bltx:volume',
  './bltx:note/preceding::bltx:start', './bltx:title/ancestor::*', './bltx:title/ancestor-or-self::bltx:entry/@id',
  './bltx:title/self::bltx:title', './bltx:title/self::bltx:note', './descendant::bltx:end',
  './child::bltx:date[2]/attribute::type', './o:title', './title', './bltx:*', './o:*',
  '/bltx:entries/bltx:entry[@id="e2"]/bltx:title', '//bltx:volume', '//bltx:volume[. > 5]',
  '//bltx:volume[. = 3]', "//bltx:entry[bltx:volume = '12']/\@id", '//bltx:entry[count(bltx:date) > 1]/@id',
  "//bltx:title[contains(., 'Sec')]", "//bltx:title[starts-with(., 'The')]",
  "//bltx:entry[\@id='e1' and \@entrytype='book']/bltx:volume", "//bltx:entry[\@id='e1' or \@id='e2']/\@id",
  'count(.//bltx:namepart)', 'string(./bltx:note)', 'normalize-space(./bltx:note)', 'string-length(./bltx:title)',
  "concat(./\@id, '-', ./bltx:volume)", "substring(./bltx:title, 2, 3)", "substring-before(./bltx:title, 'T')",
  "substring-after(./bltx:title, ' ')", "translate(./bltx:title, 'Te', 'tE')", 'sum(//bltx:volume)',
  'floor(2.5)', 'ceiling(2.5)', 'round(2.5)', 'round(-2.5)', '1 div 0', '0 div 0', '5 mod 3', '-5 mod 3',
  '2 + 3 * 4', '(2 + 3) * 4', '7 div 2', 'number("12")', 'number(" 1.5 ")', 'number("x")', 'boolean(./bltx:zz)',
  'not(./bltx:title)', 'true()', 'false()', './bltx:volume * 2', './bltx:volume = "3"', './bltx:date != 2001',
  'name(./*[1])', 'local-name(./*[1])', 'namespace-uri(./*[1])', 'name(./o:title)', 'last()', 'position()',
  './bltx:date[1] | ./bltx:title', './bltx:title[', '/[@id="x"]', './bltx:title]', './foo:bar', 'unknownfn(1)',
  './bltx:date[@type=]', '', '$var', "./bltx:names[\@type = 'author']/bltx:name[2]/bltx:namepart[\@type='family']",
  '//bltx:item[bltx:start = 10]', '//bltx:start[. >= 10]', '//bltx:start[. < "5"]', './/text()[normalize-space()]',
  './/comment()', './/processing-instruction()', 'id("e1")', 'lang("en")',
);

my $doc = XML::LibXML->load_xml(string => $xml);
my ($e1) = $doc->documentElement->findnodes('./bltx:entry');
my $json = JSON::PP->new->canonical->utf8;
for my $x (@exprs) {
  my %r = (expr => $x);
  # findnodes: node paths, or the error
  my @n = eval { $e1->findnodes($x) };
  if ($@) { $r{nodes_error} = 1 } else { $r{nodes} = [ map { $_->nodePath } @n ] }
  # findvalue: the literal value (string concatenation for node sets)
  my $v = eval { $e1->findvalue($x) };
  if ($@) { $r{value_error} = 1 } else { $r{value} = "$v" }
  # exists: boolean, or the error (-1 is true)
  my $b = eval { $e1->exists($x) ? 1 : 0 };
  if ($@) { $r{exists_error} = 1 } else { $r{exists} = $b }
  print $json->encode(\%r), "\n";
}
