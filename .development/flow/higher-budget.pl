#!/usr/bin/env perl
# Resolves a conflict in a task's own task file when the branch and main differ only in budget numbers, since the architect raises a budget on the branch while main can raise it too: the file takes main's text with the higher of each budget. Any other difference exits 1 and leaves the file alone, so merge-main stops for the architect.
# Usage: perl higher-budget.pl <task file>, inside a merge that left the file conflicted
use strict;
use warnings;

my $file = shift;
my %side;
for my $stage (2, 3) {
    open my $show, '-|', 'git', 'show', ":$stage:$file" or die "git show :$stage:$file: $!\n";
    $side{$stage} = do { local $/; <$show> };
    close $show or exit 1;
}
my %higher;
for my $key ('max_changed_lines', 'max_new_expects') {
    for my $stage (2, 3) {
        while ($side{$stage} =~ /\b$key: *(\d+)/g) {
            $higher{$key} = $1 if !defined $higher{$key} || $1 > $higher{$key};
        }
    }
}
for my $stage (2, 3) {
    for my $key (keys %higher) {
        $side{$stage} =~ s/\b$key: *\d+/$key: $higher{$key}/g;
    }
}
exit 1 if $side{2} ne $side{3};
open my $out, '>', $file or die "$file: $!\n";
print {$out} $side{3};
close $out;
