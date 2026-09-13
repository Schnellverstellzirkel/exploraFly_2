# Copyright 2021-2024, Alejandro Colomar <alx@kernel.org>
# SPDX-License-Identifier: LGPL-3.0-only WITH LGPL-3.0-linking-exception


ifndef MAKEFILE_LINT_SH_SHELLCHECK_INCLUDED
MAKEFILE_LINT_SH_SHELLCHECK_INCLUDED := 1


include $(MAKEFILEDIR)/configure/build-depends/coreutils/touch.mk
include $(MAKEFILEDIR)/configure/build-depends/shellcheck/shellcheck.mk
include $(MAKEFILEDIR)/src/sh.mk


_LINT_sh_shellcheck := $(patsubst $(SRCBINDIR)/%, $(builddir)/%.lint-sh.shellcheck.touch, $(BIN_sh))


$(_LINT_sh_shellcheck): $(builddir)/%.lint-sh.shellcheck.touch: $(SRCBINDIR)/% $(SHELLCHECK_CONF) $(MK) | $$(@D)/
	$(info	$(INFO_)SHELLCHECK	$@)
	$(SHELLCHECK) $(SHELLCHECKFLAGS_) $<
	$(TOUCH) $@


.PHONY: lint-sh-shellcheck
lint-sh-shellcheck: $(_LINT_sh_shellcheck);


endif  # include guard
