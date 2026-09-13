# Copyright 2021-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_LINT_MAN_MANDOC_INCLUDED
MAKEFILE_LINT_MAN_MANDOC_INCLUDED := 1


include $(MAKEFILEDIR)/build/_.mk
include $(MAKEFILEDIR)/build/man/man.mk
include $(MAKEFILEDIR)/build/man/mdoc.mk
include $(MAKEFILEDIR)/configure/build-depends/coreutils/touch.mk
include $(MAKEFILEDIR)/configure/build-depends/coreutils/true.mk
include $(MAKEFILEDIR)/configure/build-depends/grep/grep.mk
include $(MAKEFILEDIR)/configure/build-depends/mandoc/mandoc.mk
include $(MAKEFILEDIR)/configure/xfail.mk


_XFAIL_LINT_man_mandoc := \
	$(_MANDIR)/man7/bpf-helpers.7.lint-man.mandoc.touch \
	$(_MANDIR)/man7/uri.7.lint-man.mandoc.touch \
	$(_MANDIR)/man8/zic.8.lint-man.mandoc.touch


_LINT_man_mandoc := $(patsubst %, %.lint-man.mandoc.touch, $(_NONSO_MAN) $(_NONSO_MDOC))
ifeq ($(SKIP_XFAIL),yes)
_LINT_man_mandoc := $(filter-out $(_XFAIL_LINT_man_mandoc), $(_LINT_man_mandoc))
endif


mandoc_man_ignore_grep := $(MAKEFILEDIR)/lint/man/mandoc.ignore.grep


$(_LINT_man_mandoc): %.lint-man.mandoc.touch: % $(mandoc_man_ignore_grep) $(MK) | $$(@D)/
	$(info	$(INFO_)MANDOC		$@)
	! ($(MANDOC) $(MANDOCFLAGS_) $< 2>&1 \
	   | $(GREP) -v -f '$(mandoc_man_ignore_grep)' \
	   || $(TRUE); \
	) \
	| $(GREP) ^ >&2
	$(TOUCH) $@


.PHONY: lint-man-mandoc
lint-man-mandoc: $(_LINT_man_mandoc);


endif  # include guard
