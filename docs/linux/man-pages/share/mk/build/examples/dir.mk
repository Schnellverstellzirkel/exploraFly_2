# Copyright 2021-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_BUILD_EX_DIR_INCLUDED
MAKEFILE_BUILD_EX_DIR_INCLUDED := 1


include $(MAKEFILEDIR)/build/man/man.mk
include $(MAKEFILEDIR)/configure/build-depends/coreutils/mkdir.mk
include $(MAKEFILEDIR)/configure/build-depends/coreutils/touch.mk


_PAGEEXDIRS := $(patsubst %, %.d/, $(_NONSO_MAN))


$(_PAGEEXDIRS):
	+$(info	$(INFO_)MKDIR		$@)
	+$(MKDIR) -p $@
	+$(TOUCH) $@


.PHONY: build-ex-dir
build-ex-dir: $(_PAGEEXDIRS);


endif  # include guard
