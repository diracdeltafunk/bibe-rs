package BtProbe;
# Black-box probe of Text::BibTeX 0.91 (btparse) driven exactly the way
# biber 2.22 drives it (lib/Biber/Input/file/bibtex.pm, lib/Biber.pm).
#
# Clean-room note: this file only *calls* the public Text::BibTeX Perl API.
# It never looks at btparse internals.
#
# Run with the O3 perl:  vendor/perl/bin/perl -Itools/oracle/probes/btparse ...
use strict;
use warnings;
use utf8;
use Text::BibTeX qw(:metatypes :nameparts :joinmethods :macrosubs);
use Text::BibTeX::Name;
use Text::BibTeX::NameFormat;
use Unicode::Normalize qw(NFD NFC);
use Encode qw(encode decode);
use File::Temp qw(tempdir);
use JSON::PP;
use POSIX ();

our $DIR = tempdir(CLEANUP => 1);
our $FILE = 'test.bib';    # name btparse sees (we chdir into $DIR)
chdir $DIR or die;

my %MONTHS = (jan => '1', feb => '2', mar => '3', apr => '4', may => '5', jun => '6',
              jul => '7', aug => '8', sep => '9', oct => '10', nov => '11', dec => '12');

# --- stderr capture -------------------------------------------------------
# libbtparse writes diagnostics with C stdio to fd 2. Like biber we redirect
# STDERR to a file and read back what was appended.
my $ERRFILE = "$DIR/stderr.txt";
open(my $OLDERR, '>&', \*STDERR) or die;
open(STDERR, '>', $ERRFILE) or die;
STDERR->autoflush(1);
my $errpos = 0;
$SIG{__DIE__} = sub { print $OLDERR "DIE: @_"; exit 99 };
$SIG{__WARN__} = sub { print $OLDERR "WARN: @_" };

sub take_stderr {
  open(my $fh, '<:raw', $ERRFILE) or die;
  seek($fh, $errpos, 0);
  local $/;
  my $s = <$fh> // '';
  $errpos += length($s);
  close $fh;
  $s = decode('UTF-8', $s, Encode::FB_PERLQQ);
  return [ grep { length } split /\n/, $s ];
}

sub restore_stderr { open(STDERR, '>&', $OLDERR) }

# Run CODE in a forked child (libbtparse can abort/segfault on some inputs).
# Returns CODE's result (a JSON-able value), or {crash => signal} if the child died.
sub isolated {
  my ($code) = @_;
  pipe(my $r, my $w) or die "pipe: $!";
  my $pid = fork() // die "fork: $!";
  if (!$pid) {
    close $r;
    my $res = $code->();
    print $w JSON::PP->new->utf8->canonical->allow_nonref->encode($res);
    close $w;
    POSIX::_exit(0);
  }
  close $w;
  local $/;
  my $out = <$r>;
  close $r;
  waitpid($pid, 0);
  my $st = $?;
  if ($st != 0 or !defined $out or $out eq '') {
    my $diags = take_stderr();
    return { crash => ($st & 127) || ($st >> 8), diags => $diags };
  }
  $errpos = -s $ERRFILE;
  return JSON::PP->new->utf8->decode($out);
}

# --- macro table ------------------------------------------------------------
# biber (Biber.pm:4591) before reading datasources:
#   foreach mon (keys %MONTHS) { delete_macro(mon); add_macro_text(mon, MONTHS{mon}) }
sub reset_macros {
  my $mode = shift // 'biber';
  delete_all_macros();
  if ($mode eq 'biber') {
    for my $m (sort keys %MONTHS) { add_macro_text($m, $MONTHS{$m}) }
  }
  elsif ($mode eq 'tb') {   # Text::BibTeX load-time defaults
    my @full = qw(January February March April May June July August September October November December);
    my @ab = qw(jan feb mar apr may jun jul aug sep oct nov dec);
    add_macro_text($ab[$_], $full[$_]) for 0..11;
  }
  take_stderr();
}

sub macro_dump {
  my @names = @_;
  my %h;
  for my $n (@names) {
    my $len = macro_length($n);
    $h{$n} = $len ? decode('UTF-8', macro_text($n)) : undef;
  }
  take_stderr();
  return \%h;
}

# --- parsing a file the way biber does --------------------------------------
my %MT = (BTE_UNKNOWN() => 'unknown', BTE_REGULAR() => 'regular', BTE_COMMENT() => 'comment',
          BTE_PREAMBLE() => 'preamble', BTE_MACRODEF() => 'macrodef');

sub parse_text {
  my ($text, %o) = @_;
  my $fname = $o{file} // $FILE;
  open(my $fh, '>:raw', "$DIR/$fname") or die;
  print $fh encode('UTF-8', $text);
  close $fh;
  take_stderr();
  my $bib = Text::BibTeX::File->new();
  $bib->open($fname, {binmode => 'utf-8', normalization => 'NFD'}) or die "open: $!";
  my @entries;
  while (my $entry = Text::BibTeX::Entry->new($bib)) {
    my $mt = $MT{$entry->metatype} // ('?' . $entry->metatype);
    my %e = (metatype => $mt, ok => ($entry->parse_ok ? JSON::PP::true : JSON::PP::false));
    $e{type} = $entry->type;
    if ($mt eq 'regular') {
      $e{key} = $entry->key;
    }
    if ($mt eq 'regular' or $mt eq 'macrodef') {
      my @f;
      for my $f ($entry->fieldlist) {
        my $v = $entry->get(encode('UTF-8', NFC($f)));
        push @f, [$f, $v];
      }
      $e{fields} = \@f;
    }
    if ($mt eq 'preamble' or $mt eq 'comment') {
      $e{value} = $entry->value;
    }
    if ($mt eq 'unknown') {
      $e{print_s} = $entry->print_s;
    }
    $e{diags} = take_stderr();
    push @entries, \%e;
  }
  $bib->close;
  my $tail = take_stderr();
  return { entries => \@entries, tail => $tail };
}

# --- split_list exactly as biber's _name/_list -------------------------------
sub split_list_biber {
  my ($s, $delim) = @_;
  take_stderr();
  my @r = Text::BibTeX::split_list(NFC($s), $delim, undef, undef, undef,
                                   {binmode => 'utf-8', normalization => 'NFD'});
  return { items => [ @r ], diags => take_stderr() };
}

# split_list with explicit filename/line/description (diagnostic format)
sub split_list_full {
  my ($s, $delim, $file, $line, $desc) = @_;
  take_stderr();
  my @r = Text::BibTeX::split_list(NFC($s), $delim, $file, $line, $desc,
                                   {binmode => 'utf-8', normalization => 'NFD'});
  return { items => [ @r ], diags => take_stderr() };
}

# --- names exactly as biber's parsename (after its own pre-sanitising) -------
my %FMT;
sub _fmt {
  my ($p) = @_;
  return $FMT{$p} if $FMT{$p};
  my %part = (l => BTN_LAST, f => BTN_FIRST, v => BTN_VON, j => BTN_JR);
  my $f = Text::BibTeX::NameFormat->new($p, 0);
  $f->set_options($part{$p}, 0, BTJ_MAYTIE, BTJ_NOTHING);
  return $FMT{$p} = $f;
}

sub split_name {
  my ($s) = @_;
  take_stderr();
  my $name = Text::BibTeX::Name->new({binmode => 'utf-8', normalization => 'NFD'}, NFC($s));
  my %r;
  for my $p (qw(first von last jr)) {
    $r{$p} = [ $name->part($p) ];
  }
  # biber: family=l, given=f, prefix=v, suffix=j
  $r{fmt} = { map { $_ => $name->format(_fmt($_)) } qw(l f v j) };
  $r{diags} = take_stderr();
  return \%r;
}

# biber's own pre-sanitising in parsename (bibtex.pm:1788-1797), fixinits off
sub biber_sanitise {
  my $namestr = shift;
  $namestr =~ s/\A\s*|\s*\z//xms;
  $namestr =~ s/\s+|\\\s/ /g;
  $namestr =~ s/\A\{\{+([^\{\}]+)\}+\}\z/{$1}/xms;
  return $namestr;
}

our $JSON = JSON::PP->new->utf8->canonical->allow_nonref;

1;
