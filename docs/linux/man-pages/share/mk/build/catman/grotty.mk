# Copyright 2021-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_BUILD_CATMAN_GROTTY_INCLUDED
MAKEFILE_BUILD_CATMAN_GROTTY_INCLUDED := 1


include $(MAKEFILEDIR)/build/catman/troff.mk
include $(MAKEFILEDIR)/configure/build-depends/groff-base/grotty.mk


_CATMAN := $(patsubst %.cat.set, %.cat, $(_CATMAN_MAN_set) $(_CATMAN_MDOC_set))


$(_CATMAN): %.cat: %.cat.set $(MK) | $$(@D)/
	$(info	$(INFO_)GROTTY		$@)
	$(GROTTY) $(GROTTYFLAGS_) <$< >$@


.PHONY: build-catman-grotty
build-catman-grotty: $(_CATMAN);


endif  # include guard
