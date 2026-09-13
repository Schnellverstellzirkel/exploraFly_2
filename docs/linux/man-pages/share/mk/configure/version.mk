# Copyright 2022-2025, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_CONFIGURE_VERSION_INCLUDED
MAKEFILE_CONFIGURE_VERSION_INCLUDED := 1


include $(MAKEFILEDIR)/configure/build-depends/coreutils/echo.mk
include $(MAKEFILEDIR)/configure/build-depends/coreutils/sort.mk
include $(MAKEFILEDIR)/configure/build-depends/coreutils/stat.mk
include $(MAKEFILEDIR)/configure/build-depends/coreutils/tail.mk
include $(MAKEFILEDIR)/configure/build-depends/findutils/find.mk
include $(MAKEFILEDIR)/configure/build-depends/findutils/xargs.mk
include $(MAKEFILEDIR)/configure/build-depends/git/git.mk
include $(MAKEFILEDIR)/configure/build-depends/grep/grep.mk
include $(MAKEFILEDIR)/configure/build-depends/sed/sed.mk
include $(MAKEFILEDIR)/configure/verbose.mk


projname      := man-pages
ifndef VERSION
VERSION := 6.12
endif
ifndef EXTRAVERSION
EXTRAVERSION  :=
endif


ifndef DISTVERSION
DISTVERSION   := $(VERSION)$(EXTRAVERSION)
endif
ifndef DISTNAME
DISTNAME      := $(projname)-$(DISTVERSION)
endif


DISTFILESCMD := \
	$(FIND) $(srcdir) -not -type d \
	| $(GREP) -v '^$(srcdir)/.git$$' \
	| $(GREP) -v '^$(srcdir)/.git/' \
	| $(GREP) -v '^$(srcdir)/.tmp/' \
	| $(GREP) -v '^$(srcdir)/.checkpatch-camelcase.' \
	| $(SORT)

DISTDATECMD := \
	$(ECHO) '$(DISTVERSION)' \
	| if $(GREP) -- '-dirty$$' >/dev/null; then \
		$(DISTFILESCMD) \
		| $(XARGS) $(STAT) -c %y \
		| $(SORT) -n \
		| $(TAIL) -n1; \
	else \
		$(GIT) log -1 --format='%cD'; \
	fi;


ifndef DISTDATE
DISTDATE := Sat, 22 Feb 2025 11:38:54 +0100
endif


MANPAGEDATECMD = $(GIT) log --format=%cs -1 -- $< $(HIDE_ERR)


endif  # include guard
