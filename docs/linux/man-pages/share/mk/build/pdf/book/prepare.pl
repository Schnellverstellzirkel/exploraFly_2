#!/usr/bin/perl -w
#
#	BuildLinuxMan.pl		: Build Linux manpages book
#	Deri James (& Brian Inglis)	: 15 Dec 2022
#
#	Params:-
#
#               $1 = Directory holding the man pages
#
# (C) Copyright 2022, Deri James
#
# This program is free software; you can redistribute it and/or
# modify it under the terms of the GNU General Public License
# as published by the Free Software Foundation; either version 2
# of the License, or (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details
# (http://www.gnu.org/licenses/gpl-2.0.html).
#

use strict;
use File::Basename;

my $inTS=0;
my $inBlock=0;

my %Sections=
(
	"1"	=> "General Commands Manual",
	"2"	=> "System Calls Manual",
	"2const"	=> "System Calls Manual (constants)",
	"2type"	=> "System Calls Manual (types)",
	"3"	=> "Library Functions Manual",
	"3const"	=> "Library Functions Manual (constants)",
	"3head"	=> "Library Functions Manual (headers)",
	"3type"	=> "Library Functions Manual (types)",
	"4"	=> "Kernel Interfaces Manual",
	"5"	=> "File Formats Manual",
	"6"	=> "Games Manual",
	"7"	=> "Miscellaneous Information Manual",
	"8"	=> "System Manager's Manual",
	"9"	=> "Kernel Developer's Manual",
);

my $dir=shift || '.';
my $dir2=$dir;
$dir2=~tr[.][_];
my %files;
my %aliases;

foreach my $al (`find "$dir"/man*/ -type f \\
		| grep "\\.[[:digit:]]\\([[:alpha:]][[:alnum:]]*\\)\\?\\>\$" \\
		| xargs grep '^\\.so' /dev/null;`)
{
	#$al=~tr[.][_];
	$al=~m/^$dir\/man\d[a-z]*\/(.*):\.\s*so\s*man\d[a-z]*\/(.*)/o;

	$aliases{$1}=$2;
}

while (my ($k,$v)=each %aliases)
{
	while (exists($aliases{$v})) {
		$v=$aliases{$v};
	}
}

foreach my $fn (`find "$dir"/man*/ -type f \\
		| grep "\\.[[:digit:]]\\([[:alpha:]][[:alnum:]]*\\)\\?\\>\$";`)
{
	$fn=~s/\n//;

	my ($nm,$sec)=GetNmSec($fn,qr/\.\d[a-z]*/);
	$files{"${nm}.$sec"}=[$fn,(exists($aliases{"${nm}.$sec"}))?$aliases{"${nm}.$sec"}:"${nm}.$sec"];
}

my $Section='';

BuildBook();

sub BuildBook
{
	print ".pdfpagenumbering D . 1\n.nr PDFOUTLINE.FOLDLEVEL 0\n.defcolor pdf:href.colour rgb 0.00 0.25 0.75\n.pdfinfo /Title \"GNU/Linux Programmer's Manual\"\n.special TinosR S\n";

	foreach my $bkmark (sort sortman keys %files) {
		BuildPage($bkmark);
	}
}

sub BuildPage
{
	my $bkmark=shift;

	my $fn=$files{$bkmark}->[0];
	my ($nm,$sec,$srt)=GetNmSec($bkmark,qr/\.[\da-z]+/);

	my $title= "$nm\\($sec\\)";

	print ".\\\" >>>>>> $nm($sec) <<<<<<\n.lf 0 $bkmark\n";

	# If this is an alias, just add it to the outline panel.

	# if new section add top level bookmark

	if ($sec ne $Section) {
		print ".nr PDFOUTLINE.FOLDLEVEL 1\n";
		print ".pdfbookmark 1 $Sections{$sec}\n";
		print ".nr PDFOUTLINE.FOLDLEVEL 2\n";
		$Section=$sec;
	}

	if (exists($aliases{$bkmark})) {
		print ".eo\n.device ps:exec [/Dest /$aliases{$bkmark} /Title ($title) /Level 2 /OUT pdfmark\n.ec\n.fl\n";
		return;
	}

	if (open(F,'<',$fn)) {
		while (<F>) {
			if (m/^\.\\"/) {
				print $_;
				next;
			}

			chomp;

			# This code is to determine whether we are within a tbl block and in a text block
			# T{ and T}. This is fudge code particularly for the syscalls(7) page.

			$inTS=1 if m/\.TS/;
			$inTS=0,$inBlock=0 if m/\.TE/;

			next if !$_;
#			s/^\s+//;

			s/\\-/-/g if /^\.[BM]R\s+/;

			if (m/^\.BR\s+([-\w\\.]+)\s+\((.+?)\)(.*)/ or m/^\.MR\s+([-\w\\.]+)\s+(\w+)\s+(.*)/ or m/^\\fB([-\w\\.]+)\\fR\((.+?)\)(.*)$/) {
				my $bkmark="$1";
				my $sec=$2;
				my $after=$3;
				$after=~s/\s\\".*//;
				my $dest=$bkmark;
				$dest=~s/\\-/-/g;

				if (exists($files{"${bkmark}.$sec"})) {
					my $dest=$files{"${bkmark}.$sec"}->[1];
					$_=".pdfhref L -D \"$dest\" -A \"$after\" -- \\fI$bkmark\\fP($sec)";
				} else {
					$_=".IR $bkmark ($sec)\\c\n$after";
				}
			}

			s/^\.BI \\fB/.BI /;
			s/^\.BR\s+(\S+)\s*$/.B $1/;
			s/^\.BI\s+(\S+)\s*$/.B $1/;
			s/^\.IR\s+(\S+)\s*$/.I $1/;

			# Fiddling for syscalls(7) :-(

			if ($inTS) {
				my @cols=split(/\t/,$_);

				foreach my $c (@cols) {
					$inBlock+=()=$c=~m/T\{/g;
					$inBlock-=()=$c=~m/T\}/g;

					my $mtch=$c=~s/\s*\\fB([-\w.]+)\\fP\((\w+)\)/doMR($1,$2)/ge;
					$c="T{\n${c}\nT}" if $mtch and !$inBlock;
				}

				$_=join("\t",@cols);
				s/\n\n/\n/g;
			}

			s/\\&\././ if m/^.TH /;

			if (m/^\.TH\s+"?([-\w\\.]+)"?\s+"?(\w+)"?/) {

				print "$_\n";

				# Add a level two bookmark. We don't set it in the TH macro since the name passed
				# may be different from the filename, i.e. file = unimplemented.2, TH = UNIMPLEMENTED 2

				print ".pdfhref M -N $bkmark\n";

				next;
			}
			print "$_\n";
		}
		close(F);
	}
}

sub doMR
{
	my $nm=shift;
	my $sec=shift;

	if (exists($files{"${nm}.$sec"})) {
		return("\n.pdfhref L -D \"$files{\"${nm}.$sec\"}->[1]\" -A \"\\c\" -- \\fI$nm\\fP($sec)\n");
	} else {
		return("\\fI$nm\\fP($sec)");
	}
}

sub GetNmSec
{
	my ($nm,$pth,$sec)=fileparse($_[0],$_[1]);
	$sec=substr($sec,1);
	my $srt=$nm;
	$srt=~s/\..+?$//;
	$srt=~s/^_+//;
	$srt=$1.sprintf("%04d",$2) if $srt=~m/^(.+)(\d+)$/;
	#$srt="$sec/$srt";
	return($nm,$sec,$srt);
}

# add rpmvercmp
#use RPM::VersionSort;
#use Sort::Versions;

sub sortman
{
# Sort - ignore case but frig it so that intro is the first entry.

	my (undef,$s1,$c)=GetNmSec($a,qr/\.\d[a-z]*/);
	my (undef,$s2,$d)=GetNmSec($b,qr/\.\d[a-z]*/);

	my $cmp=$s1 cmp $s2;

	return $cmp if $cmp;
	return -1 if ($c=~m/^intro/ and $d!~m/^intro/);
	return  1 if ($d=~m/^intro/ and $c!~m/^intro/);
	$c=~tr[-_(][!" ];
	$d=~tr[-_(][!" ];
	$cmp=lc($c) cmp lc($d);
	return($c cmp $d) if $cmp == 0;
	return($cmp);
}

sub strhex
{
	my $res='';

	foreach my $c (split('',$_[0])) {
		$res.=sprintf("%02X",ord($c));
	}

	return($res);
}
