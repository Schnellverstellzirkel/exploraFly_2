# Copyright 2021-2025, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


SHELL       := bash
.SHELLFLAGS := -Eeuo pipefail -c


ifneq (4.4.999,$(firstword $(sort 4.4.999 $(MAKE_VERSION))))
  ifneq (R,$(findstring R, $(firstword -$(MAKEFLAGS))))
    $(error Please run make(1) with the '-R' option)
  endif
endif


MAKEFLAGS += --no-builtin-rules
MAKEFLAGS += --no-builtin-variables
MAKEFLAGS += --warn-undefined-variables


ifndef srcdir
srcdir      := .
endif
MAKEFILEDIR := $(CURDIR)/share/mk


INFO_ :=


.PHONY: all
all: build;


.SECONDEXPANSION:


MK_ := $(wildcard $(addprefix $(MAKEFILEDIR)/, *.mk */*.mk */*/*.mk */*/*/*.mk))
MK  := $(CURDIR)/GNUmakefile $(MK_)
include $(MK_)
$(MK):: ;


.PHONY: nothing
nothing:;


.PHONY: help
help:
	$(info	$(INFO_)To see a list of targets, run:)
	$(info	$(INFO_)	$$ make -R -p nothing \)
	$(info	$(INFO_)	| grep '^\.PHONY:' \)
	$(info	$(INFO_)	| tr ' ' '\n' \)
	$(info	$(INFO_)	| grep -v '^\.PHONY:' \)
	$(info	$(INFO_)	| sort;)
	$(info	)
	$(info	$(INFO_)To see a list of variables, run:)
	$(info	$(INFO_)	$$ find GNUmakefile share/mk/configure -type f \)
	$(info	$(INFO_)	| sort \)
	$(info	$(INFO_)	| xargs grep '^[^[:space:]].*?=' \)
	$(info	$(INFO_)	| sed 's/=.*/=/';)
	$(info	)
	$(info	$(INFO_)To see a list of dependencies (package/program), run:)
	$(info	$(INFO_)	$$ find share/mk/configure/build-depends -type f \)
	$(info	$(INFO_)	| sed 's,share/mk/configure/build-depends/,,' \)
	$(info	$(INFO_)	| sed 's,\.mk,,' \)
	$(info	$(INFO_)	| sort;)
	$(info	)


.DELETE_ON_ERROR:
.SILENT:
FORCE:
