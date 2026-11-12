no warnings; use strict;
my $want = shift;
my (%st, %file, %rev, %h);
for my $f (glob "backlog/*.yaml") {
  open my $fh, "<", $f or die; local $/; my $t = <$fh>;
  my ($id) = $t =~ /^id: (\S+)/m; ($st{$id}) = $t =~ /^status: (\S+)/m; ($file{$id}) = $t =~ /^file: (\S+)/m;
  my ($d) = $t =~ /^depends_on: \[(.*?)\]/m;
  push @{$rev{$_}}, $id for grep { length } split /,\s*/, $d // "";
}
# same height as free.pl: open tasks chained behind, where an umbrella without a task file adds no step
sub height {
  my $x = shift;
  return $h{$x} if exists $h{$x};
  $h{$x} = 0;
  my $m = 0;
  for my $c (@{$rev{$x} // []}) { next if $st{$c} eq "done"; my $v = height($c); $m = $v if $v > $m }
  $h{$x} = $m + (($file{$x} // "null") eq "null" ? 0 : 1);
}
my $top = 0;
for my $id (keys %st) { next if $st{$id} eq "done"; my $v = height($id); $top = $v if $v > $top }
print height($want), " $top\n";
