# Copyright 2021-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_BUILD_PRE_TBL_INCLUDED
MAKEFILE_BUILD_PRE_TBL_INCLUDED := 1


include $(MAKEFILEDIR)/build/pre/preconv.mk
include $(MAKEFILEDIR)/configure/build-depends/groff-base/tbl.mk


_MAN_eqn := $(patsubst %.tbl, %.eqn, $(_MAN_tbl))


$(_MAN_eqn): %.eqn: %.tbl $(MK) | $$(@D)/
	$(info	$(INFO_)TBL		$@)
	$(TBL) <$< >$@


.PHONY: build-pre-tbl
build-pre-tbl: $(_MAN_eqn);


endif  # include guard
