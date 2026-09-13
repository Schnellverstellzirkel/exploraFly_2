# Copyright 2021-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_BUILD_MAN_MDOC_INCLUDED
MAKEFILE_BUILD_MAN_MDOC_INCLUDED := 1


include $(MAKEFILEDIR)/build/_.mk
include $(MAKEFILEDIR)/configure/build-depends/coreutils/cp.mk
include $(MAKEFILEDIR)/configure/directory_variables/src.mk
include $(MAKEFILEDIR)/src/man.mk


_NONSO_MDOC := $(patsubst $(MANDIR)/%, $(_MANDIR)/%, $(NONSO_MDOC))


$(_NONSO_MDOC): $(_MANDIR)/%: $(MANDIR)/% $(MK) | $$(@D)/
	$(info	$(INFO_)CP		$@)
	$(CP) -T $< $@


.PHONY: build-man-mdoc
build-man-mdoc: $(_NONSO_MDOC)


endif  # include guard
