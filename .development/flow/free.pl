my ($skip, @extra) = @ARGV;
my (%st,%file,%deps,%scope);
for my $f (glob "backlog/*.yaml") { open my $h,"<",$f; local $/; my $t=<$h>; my ($id)=$t=~/^id: (\S+)/m;
  ($st{$id})=$t=~/^status: (\S+)/m; ($file{$id})=$t=~/^file: (\S+)/m; my ($d)=$t=~/^depends_on: \[(.*?)\]/m; $deps{$id}=[grep {length} split /,\s*/, $d//""]; }
for my $id (keys %file) { next if $file{$id} eq "null"; open my $h,"<",$file{$id} or next; local $/; my $t=<$h>;
  my ($s)=$t=~/^scope:\n((?:  .*\n)+)/m; my ($p)=$s=~/^  paths:\n((?:  [ -].*\n)+)/m; $scope{$id}=[map {s/^\s+- //; s/^"|"$//g; $_} grep {/^\s+- /} split /\n/, $p//""]; }
$st{$_}="in-progress" for @extra;
if (open my $r, "<", "$ENV{BETTERLITE_LOGS}/feeder.reserve") { while (<$r>) { chomp; $st{$_}="in-progress" if length } }
sub glob_re { my $g=shift; my $re=""; while (length $g) {
    if ($g=~s{^\*\*/}{}) { $re.="(?:.*/)?" } elsif ($g=~s{^/\*\*$}{}) { $re.="/.*" } elsif ($g=~s{^\*\*}{}) { $re.=".*" }
    elsif ($g=~s{^\*}{}) { $re.="[^/]*" } elsif ($g=~s{^\?}{}) { $re.="[^/]" } elsif ($g=~s{^(\[[^\]]*\])}{}) { $re.=$1 }
    elsif ($g=~s#^\{([^}]*)\}##) { $re.="(?:".join("|", map { glob_re($_) } split /,/, $1).")" }
    else { $g=~s{^(.)}{}s; $re.=quotemeta $1 } } $re }
sub set_re { my @p=@_; return qr/(?!)/ unless @p; my $alt=join "|", map { glob_re($_) } @p; qr/^(?:$alt)$/ }
my @always; if (open my $c, "<", "checks.yaml") { my $in=0; while (<$c>) { if (/^  (always_in_scope|overlap_allowed):/) { $in=1; next } if ($in) { if (/^    - "?([^"\n]+)"?\s*$/) { push @always, $1 } else { $in=0 } } } }
my $shared=set_re(@always);
my @tracked=split /\n/, `git -C .. ls-files`;
sub probes { my @out; for my $p (@_) { if ($p=~/^(.*?)\{([^}]*)\}(.*)$/) { push @out, map { "$1$_$3" } split /,/, $2 } else { push @out, $p } }
  map { my $x=$_; $x=~s/[*?\[].*/probe/s; $x } @out }
my (%re,%hits,%probe);
sub prep { my $id=shift; return if $re{$id}; $re{$id}=set_re(@{$scope{$id}//[]}); my $r=$re{$id}; $hits{$id}={ map { $_=>1 } grep { $_=~$r && $_!~$shared } @tracked }; $probe{$id}=[grep { $_!~$shared } probes(@{$scope{$id}//[]})] }
sub hit { my ($a,$b)=@_; prep($a); prep($b); my ($ha,$hb)=($hits{$a},$hits{$b}); ($ha,$hb)=($hb,$ha) if keys %$ha > keys %$hb;
  for my $f (sort keys %$ha) { return $f if $hb->{$f} } for my $p (@{$probe{$a}}, @{$probe{$b}}) { return $p if $p=~$re{$a} && $p=~$re{$b} } "" }
my %unblock; for my $id (keys %deps){ $unblock{$_}++ for @{$deps{$id}} }
my %rev; for my $id (keys %deps){ push @{$rev{$_}}, $id for @{$deps{$id}} }
my %height; sub height { my $x=shift; return $height{$x} if exists $height{$x}; $height{$x}=0; my $m=0; for my $c (@{$rev{$x}//[]}) { next if $st{$c} eq "done"; my $v=height($c); $m=$v if $v>$m } $height{$x}=$m+(($file{$x}//"null") eq "null" ? 0 : 1) }
my @run=grep { $st{$_} eq "in-progress" } keys %st;
my %hold; if (open my $h, "<", "$ENV{BETTERLITE_LOGS}/feeder.hold") { while (<$h>) { chomp; $hold{$_}=1 if length } }
sub owns { my ($id,$path)=@_; prep($id); $path=~$re{$id} }
sub undeclared { my $id=shift; my @mods=grep { !/[*?\[{]/ } @{$scope{$id}}; push @mods, map { m{^(.*/src/[a-z0-9_/]+)/\*\*$} ? "$1.rs" : () } @{$scope{$id}};
  for my $p (@mods) { next if -e "../$p"; my ($src,$rel)=$p=~m{^(.*/src)/(.+)\.rs$} or next; next if $rel=~m{^(lib|main|build)$|^bin/};
    my ($dir,$name)=$rel=~m{^(?:(.*)/)?([^/]+)$}; my @parents=defined $dir ? ("$src/$dir.rs","$src/$dir/mod.rs") : ("$src/lib.rs","$src/main.rs");
    next if grep { owns($id,$_) } @parents; my $ok=0;
    for my $par (@parents) { open my $h,"<","../$par" or next; local $/; my $t=<$h>; $ok=1 if $t=~/^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+\Q$name\E\s*;/m || $t=~/=>\s*\Q$name\E\s*,/ || $t=~/^\s+(?:[a-z0-9_]+,\s+)*\Q$name\E,/m }
    return "$p needs mod $name in $parents[0]" unless $ok } "" }
my @ready=sort { height($b) <=> height($a) || ($unblock{$b}//0) <=> ($unblock{$a}//0) || $a cmp $b } grep { !$hold{$_} && $st{$_} eq "todo" && $file{$_} ne "null" && !grep { $st{$_} ne "done" } @{$deps{$_}} } keys %st;
for my $id (@ready){ my $u=undeclared($id); if($u){ print "undeclared $id $u\n"; next } my @h=grep { hit($id,$_) } @run; if(!@h){ push @run,$id; print "free $id ",height($id),"\n" } else { print "wait $id ",height($id)," on @h: ",hit($id,$h[0]),"\n" } }
