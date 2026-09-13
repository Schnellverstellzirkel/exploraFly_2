# Copyright 2021-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_BUILD_MAN_SO_INCLUDED
MAKEFILE_BUILD_MAN_SO_INCLUDED := 1


include $(MAKEFILEDIR)/build/_.mk
include $(MAKEFILEDIR)/configure/build-depends/coreutils/cp.mk
include $(MAKEFILEDIR)/configure/directory_variables/src.mk
include $(MAKEFILEDIR)/src/man.mk


_SO_MAN := $(patsubst $(MANDIR)/%, $(_MANDIR)/%, $(SO_MAN))


$(_SO_MAN): $(_MANDIR)/%: $(MANDIR)/% $(MK) | $$(@D)/
	$(info	$(INFO_)CP		$@)
	$(CP) -T $< $@


.PHONY: build-man-so
build-man-so: $(_SO_MAN)


endif  # include guard
