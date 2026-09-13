# Copyright 2021-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_LINT_C_CPPLINT_INCLUDED
MAKEFILE_LINT_C_CPPLINT_INCLUDED := 1


include $(MAKEFILEDIR)/build/examples/src.mk
include $(MAKEFILEDIR)/configure/build-depends/coreutils/touch.mk
include $(MAKEFILEDIR)/configure/build-depends/cpplint/cpplint.mk


_LINT_c_EX_cpplint   := $(patsubst %, %.lint-c.cpplint.touch, $(_EX_TU_src))
_LINT_c_cpplint      := $(_LINT_c_EX_cpplint)


$(_LINT_c_EX_cpplint): %.lint-c.cpplint.touch: %
$(_LINT_c_cpplint): $(CPPLINT_CONF) $(MK) | $$(@D)/


$(_LINT_c_cpplint):
	$(info	$(INFO_)CPPLINT		$@)
	$(CPPLINT) $(CPPLINTFLAGS_) $< >/dev/null
	$(TOUCH) $@


.PHONY: lint-c-cpplint
lint-c-cpplint: $(_LINT_c_cpplint);


endif  # include guard
