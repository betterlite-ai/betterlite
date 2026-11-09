no warnings; use strict;
my $want = shift;
my (%st, %rev, %h);
for my $f (glob "backlog/*.yaml") {
  open my $fh, "<", $f or die; local $/; my $t = <$fh>;
  my ($id) = $t =~ /^id: (\S+)/m; ($st{$id}) = $t =~ /^status: (\S+)/m;
  my ($d) = $t =~ /^depends_on: \[(.*?)\]/m;
  push @{$rev{$_}}, $id for grep { length } split /,\s*/, $d // "";
}
sub height {
  my $x = shift;
  return $h{$x} if exists $h{$x};
  $h{$x} = 0;
  my $m = 0;
  for my $c (@{$rev{$x} // []}) { next if $st{$c} eq "done"; my $v = height($c); $m = $v if $v > $m }
  $h{$x} = $m + 1;
}
print height($want), "\n";
