# Copyright 2021-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_BUILD_PRE_PRECONV_INCLUDED
MAKEFILE_BUILD_PRE_PRECONV_INCLUDED := 1


include $(MAKEFILEDIR)/build/man/man.mk
include $(MAKEFILEDIR)/build/man/mdoc.mk
include $(MAKEFILEDIR)/configure/build-depends/groff-base/preconv.mk


_MAN_tbl := $(patsubst %, %.tbl, $(_NONSO_MAN) $(_NONSO_MDOC))


$(_MAN_tbl): %.tbl: % $(MK) | $$(@D)/
	$(info	$(INFO_)PRECONV		$@)
	$(PRECONV) $(PRECONVFLAGS_) $< >$@


.PHONY: build-pre-preconv
build-pre-preconv: $(_MAN_tbl);


endif  # include guard
