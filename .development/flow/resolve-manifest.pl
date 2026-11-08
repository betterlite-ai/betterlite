#!/usr/bin/env perl
# Resolves dependency-line conflicts in one Cargo.toml: one line per dependency name; main's line wins, with the feature lists merged when only features differ. In a fuzz manifest whose only open conflict is the target list, the [[bin]] entries of both sides are kept, main's first. Any other hunk with a section header, a blank line or a name or path key keeps its markers, so merge-main stops for a person.
# Usage: perl resolve-manifest.pl <Cargo.toml>
use strict;
use warnings;

my $file = shift;
open my $in, '<', $file or die "$file: $!\n";
my $text = do { local $/; <$in> };
close $in;
$text =~ s{<<<<<<< HEAD\n(.*?)=======\n(.*?)>>>>>>> origin/main\n}{resolve($1, $2)}sge;
$text = union_bins($file, $text) if $file =~ m{(^|/)fuzz/Cargo\.toml$} && $text =~ /^<<<<<<< /m;
open my $out, '>', $file or die "$file: $!\n";
print {$out} $text;
close $out;

sub resolve {
    my ($ours, $theirs) = @_;
    my @lines = (split(/\n/, $ours), split(/\n/, $theirs));
    return "<<<<<<< HEAD\n$ours=======\n$theirs>>>>>>> origin/main\n" if grep { /^\[/ || !/\S/ || /^(name|path) = / } @lines;
    my %line;
    for my $l (@lines) {
        my ($key) = $l =~ /^([A-Za-z0-9_-]+)/;
        $key //= $l;
        my $earlier = $line{$key};
        $line{$key} = defined $earlier ? merged($earlier, $l) : $l;
    }
    return join '', map { "$line{$_}\n" } sort keys %line;
}

# two specs of one dependency that differ only in features keep every feature either side asked for; otherwise main's spec wins
sub merged {
    my ($ours, $theirs) = @_;
    my $pattern = qr/features = \[([^\]]*)\]/;
    my ($our_features) = $ours =~ $pattern;
    my ($their_features) = $theirs =~ $pattern;
    return $ours if defined $our_features && !defined $their_features && bare($ours) eq bare($theirs);
    return $theirs unless defined $our_features && defined $their_features;
    return $theirs unless ($ours =~ s/$pattern//r) eq ($theirs =~ s/$pattern//r);
    my @features = map { s/^\s+|\s+$//gr } split /,/, "$their_features,$our_features";
    my %seen;
    my $list = join ', ', grep { length && !$seen{$_}++ } @features;
    return $theirs =~ s/$pattern/features = [$list]/r;
}

# a spec without its features, with name.workspace = true written as name = { workspace = true }
sub bare {
    my ($spec) = @_;
    $spec =~ s/,?\s*features = \[[^\]]*\]//;
    $spec =~ s/^([A-Za-z0-9_-]+)\.workspace = true$/$1 = { workspace = true }/;
    return $spec;
}

# the head before the first [[bin]] must be clean; each target comes once by name, main's copy when both sides have it
sub union_bins {
    my ($file, $text) = @_;
    my ($head) = $text =~ /\A(.*?)^\[\[bin\]\]\n/ms;
    return $text if !defined $head || $head =~ /^(<<<<<<<|=======|>>>>>>>)/m;
    my @blocks;
    my %seen;
    for my $stage (3, 2) {
        my $side = `git show :$stage:$file`;
        return $text if $?;
        my (undef, @bins) = split /^\[\[bin\]\]\n/m, $side;
        for my $bin (@bins) {
            return $text if $bin =~ /^\[/m;
            my ($name) = $bin =~ /^name = "([^"]+)"/m;
            return $text unless defined $name;
            next if $seen{$name}++;
            $bin =~ s/\s+\z/\n/;
            push @blocks, "[[bin]]\n$bin";
        }
    }
    return $head . join("\n", @blocks);
}
